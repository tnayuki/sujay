pub(crate) const PEAK_BINS: u32 = 1024;
pub(crate) const COMPUTE_WORKGROUP_SIZE: u32 = 64;
pub(crate) const WAVEFORM_SHADER: &str = include_str!("waveform.wgsl");

pub(crate) struct DeckGpuState {
  pub(crate) sample_buffer: wgpu::Buffer,
  pub(crate) peak_buffer: wgpu::Buffer,
  pub(crate) compute_params_buffer: wgpu::Buffer,
  pub(crate) compute_bind_group: wgpu::BindGroup,
  pub(crate) sample_count: u32,
}

pub(crate) struct RendererResources {
  pub(crate) compute_bind_group_layout: wgpu::BindGroupLayout,
  pub(crate) compute_pipeline: wgpu::ComputePipeline,
  pub(crate) render_pipeline: wgpu::RenderPipeline,
  pub(crate) render_params_buffer: wgpu::Buffer,
  pub(crate) deck_states: [DeckGpuState; 2],
  pub(crate) render_bind_group: wgpu::BindGroup,
}

pub(crate) fn encode_u32_f32_f32_f32(a: u32, b: f32, c: f32, d: f32) -> [u8; 16] {
  let mut out = [0u8; 16];
  out[0..4].copy_from_slice(&a.to_ne_bytes());
  out[4..8].copy_from_slice(&b.to_bits().to_ne_bytes());
  out[8..12].copy_from_slice(&c.to_bits().to_ne_bytes());
  out[12..16].copy_from_slice(&d.to_bits().to_ne_bytes());
  out
}

pub(crate) fn encode_u32x4(a: u32, b: u32, c: u32, d: u32) -> [u8; 16] {
  let mut out = [0u8; 16];
  out[0..4].copy_from_slice(&a.to_ne_bytes());
  out[4..8].copy_from_slice(&b.to_ne_bytes());
  out[8..12].copy_from_slice(&c.to_ne_bytes());
  out[12..16].copy_from_slice(&d.to_ne_bytes());
  out
}

pub(crate) fn choose_surface_config(
  surface: &wgpu::Surface<'_>,
  adapter: &wgpu::Adapter,
  width: u32,
  height: u32,
) -> wgpu::SurfaceConfiguration {
  let caps = surface.get_capabilities(adapter);
  let format = caps
    .formats
    .iter()
    .copied()
    .find(|f| f.is_srgb())
    .unwrap_or(caps.formats[0]);

  let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
    wgpu::PresentMode::Fifo
  } else {
    caps.present_modes[0]
  };

  wgpu::SurfaceConfiguration {
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    format,
    width: width.max(1),
    height: height.max(1),
    present_mode,
    alpha_mode: caps.alpha_modes[0],
    view_formats: vec![],
    desired_maximum_frame_latency: 2,
  }
}

fn create_deck_state(
  device: &wgpu::Device,
  compute_bind_group_layout: &wgpu::BindGroupLayout,
  deck_index: usize,
) -> DeckGpuState {
  let empty_sample_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some(if deck_index == 0 {
      "sujay-native-ui-empty-samples-a"
    } else {
      "sujay-native-ui-empty-samples-b"
    }),
    size: 4,
    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
  });

  let peak_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some(if deck_index == 0 {
      "sujay-native-ui-peaks-a"
    } else {
      "sujay-native-ui-peaks-b"
    }),
    size: (PEAK_BINS as u64) * 4,
    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
  });

  let compute_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("sujay-native-ui-compute-params"),
    size: 16,
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
  });

  let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("sujay-native-ui-compute-bg"),
    layout: compute_bind_group_layout,
    entries: &[
      wgpu::BindGroupEntry {
        binding: 0,
        resource: empty_sample_buffer.as_entire_binding(),
      },
      wgpu::BindGroupEntry {
        binding: 1,
        resource: peak_buffer.as_entire_binding(),
      },
      wgpu::BindGroupEntry {
        binding: 2,
        resource: compute_params_buffer.as_entire_binding(),
      },
    ],
  });

  DeckGpuState {
    sample_buffer: empty_sample_buffer,
    peak_buffer,
    compute_params_buffer,
    compute_bind_group,
    sample_count: 0,
  }
}

pub(crate) fn create_renderer_resources(
  device: &wgpu::Device,
  shader: &wgpu::ShaderModule,
  surface_format: wgpu::TextureFormat,
) -> RendererResources {
  let compute_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
    label: Some("sujay-native-ui-compute-bgl"),
    entries: &[
      wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Storage { read_only: true },
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      },
      wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Storage { read_only: false },
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      },
      wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Uniform,
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      },
    ],
  });

  let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("sujay-native-ui-compute-layout"),
    bind_group_layouts: &[&compute_bind_group_layout],
    push_constant_ranges: &[],
  });

  let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
    label: Some("sujay-native-ui-peaks-compute"),
    layout: Some(&compute_pipeline_layout),
    module: shader,
    entry_point: Some("cs_peaks"),
    compilation_options: wgpu::PipelineCompilationOptions::default(),
    cache: None,
  });

  let render_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
    label: Some("sujay-native-ui-render-bgl"),
    entries: &[
      wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Storage { read_only: true },
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      },
      wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Storage { read_only: true },
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      },
      wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Uniform,
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      },
    ],
  });

  let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("sujay-native-ui-render-layout"),
    bind_group_layouts: &[&render_bind_group_layout],
    push_constant_ranges: &[],
  });

  let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("sujay-native-ui-waveform-render"),
    layout: Some(&render_pipeline_layout),
    vertex: wgpu::VertexState {
      module: shader,
      entry_point: Some("vs_fullscreen"),
      buffers: &[],
      compilation_options: wgpu::PipelineCompilationOptions::default(),
    },
    fragment: Some(wgpu::FragmentState {
      module: shader,
      entry_point: Some("fs_waveform"),
      targets: &[Some(wgpu::ColorTargetState {
        format: surface_format,
        blend: Some(wgpu::BlendState::REPLACE),
        write_mask: wgpu::ColorWrites::ALL,
      })],
      compilation_options: wgpu::PipelineCompilationOptions::default(),
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      ..Default::default()
    },
    depth_stencil: None,
    multisample: wgpu::MultisampleState::default(),
    multiview: None,
    cache: None,
  });

  let render_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("sujay-native-ui-render-params"),
    size: 16,
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
  });

  let deck_states = [
    create_deck_state(device, &compute_bind_group_layout, 0),
    create_deck_state(device, &compute_bind_group_layout, 1),
  ];

  let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("sujay-native-ui-render-bg"),
    layout: &render_bind_group_layout,
    entries: &[
      wgpu::BindGroupEntry {
        binding: 0,
        resource: deck_states[0].peak_buffer.as_entire_binding(),
      },
      wgpu::BindGroupEntry {
        binding: 1,
        resource: deck_states[1].peak_buffer.as_entire_binding(),
      },
      wgpu::BindGroupEntry {
        binding: 2,
        resource: render_params_buffer.as_entire_binding(),
      },
    ],
  });

  RendererResources {
    compute_bind_group_layout,
    compute_pipeline,
    render_pipeline,
    render_params_buffer,
    deck_states,
    render_bind_group,
  }
}

pub(crate) fn sync_deck_waveforms<F>(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  compute_bind_group_layout: &wgpu::BindGroupLayout,
  deck_states: &mut [DeckGpuState; 2],
  current_versions: [u64; 2],
  last_versions: &mut [u64; 2],
  mut get_samples: F,
) where
  F: FnMut(usize) -> Vec<f32>,
{
  for deck_index in 0..2 {
    let version = current_versions[deck_index];
    if version == last_versions[deck_index] {
      continue;
    }

    let samples = get_samples(deck_index);
    let sample_count = samples.len() as u32;
    let upload_samples = if samples.is_empty() {
      vec![0.0_f32]
    } else {
      samples
    };

    let bytes: Vec<u8> = upload_samples
      .iter()
      .flat_map(|value| value.to_ne_bytes())
      .collect();

    let sample_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("sujay-native-ui-sample-buffer"),
      size: bytes.len() as u64,
      usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(&sample_buffer, 0, &bytes);

    let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("sujay-native-ui-compute-bg"),
      layout: compute_bind_group_layout,
      entries: &[
        wgpu::BindGroupEntry {
          binding: 0,
          resource: sample_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 1,
          resource: deck_states[deck_index].peak_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 2,
          resource: deck_states[deck_index].compute_params_buffer.as_entire_binding(),
        },
      ],
    });

    deck_states[deck_index].sample_buffer = sample_buffer;
    deck_states[deck_index].compute_bind_group = new_bind_group;
    deck_states[deck_index].sample_count = sample_count;
    last_versions[deck_index] = version;
  }
}

pub(crate) fn write_compute_params(queue: &wgpu::Queue, deck_states: &[DeckGpuState; 2]) {
  for deck in deck_states.iter() {
    let params = encode_u32x4(deck.sample_count, PEAK_BINS, 0, 0);
    queue.write_buffer(&deck.compute_params_buffer, 0, &params);
  }
}
