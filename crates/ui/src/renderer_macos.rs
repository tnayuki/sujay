use cocoa::base::{id, nil};
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use crate::renderer_wgpu_shared::{
  choose_surface_config, create_renderer_resources, encode_u32_f32_f32_f32,
  sync_deck_waveforms, write_compute_params, RendererResources, COMPUTE_WORKGROUP_SIZE,
  PEAK_BINS, WAVEFORM_SHADER,
};
use crate::ui_state::{ConsoleVisualState, DeckConsoleVisualState};
use objc::runtime::{Class, Object};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{
  AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

// Wrapper to make raw pointer Send+Sync for static storage
struct ViewPtr(*mut Object);
unsafe impl Send for ViewPtr {}
unsafe impl Sync for ViewPtr {}

static CHILD_VIEW: Mutex<Option<ViewPtr>> = Mutex::new(None);

#[derive(Clone, Default)]
struct DeckVisualState {
  progress: f32,
  duration: f32,
  beats: Vec<f32>,
  intro: Option<f32>,
  outro: Option<f32>,
}

struct RendererState {
  running: Arc<AtomicBool>,
  pending_size: Arc<Mutex<(u32, u32)>>,
  thread: Option<JoinHandle<()>>,
}

static RENDERER: Mutex<Option<RendererState>> = Mutex::new(None);
static WAVEFORMS: Mutex<[Vec<f32>; 2]> = Mutex::new([Vec::new(), Vec::new()]);
static DECK_VISUALS: Mutex<[DeckVisualState; 2]> = Mutex::new([
  DeckVisualState {
    progress: 0.0,
    duration: 0.0,
    beats: Vec::new(),
    intro: None,
    outro: None,
  },
  DeckVisualState {
    progress: 0.0,
    duration: 0.0,
    beats: Vec::new(),
    intro: None,
    outro: None,
  },
]);
static CONSOLE_VISUAL: Mutex<ConsoleVisualState> = Mutex::new(ConsoleVisualState {
  deck_a: DeckConsoleVisualState {
    title: String::new(),
    time_text: String::new(),
    bpm_text: String::new(),
    playing: false,
    loop_enabled: false,
    loop_beats: 0.0,
    cue_enabled: false,
    eq_low: false,
    eq_mid: false,
    eq_high: false,
    gain: 1.0,
    peak: 0.0,
  },
  deck_b: DeckConsoleVisualState {
    title: String::new(),
    time_text: String::new(),
    bpm_text: String::new(),
    playing: false,
    loop_enabled: false,
    loop_beats: 0.0,
    cue_enabled: false,
    eq_low: false,
    eq_mid: false,
    eq_high: false,
    gain: 1.0,
    peak: 0.0,
  },
  master_tempo: 130.0,
  crossfader: 0.5,
});
static WAVEFORM_VERSIONS: [AtomicU64; 2] = [AtomicU64::new(1), AtomicU64::new(1)];


unsafe fn html_frame_to_nsview_frame(parent_view: id, x: f64, y: f64, width: f64, height: f64) -> NSRect {
  let parent_bounds: NSRect = msg_send![parent_view, bounds];
  let converted_y = parent_bounds.size.height - y - height;
  NSRect::new(
    NSPoint::new(x.max(0.0), converted_y),
    NSSize::new(width.max(0.0), height.max(0.0)),
  )
}

unsafe fn view_contents_scale(parent_view: id) -> f64 {
  let window: id = msg_send![parent_view, window];
  if window == nil {
    return 1.0;
  }
  let scale: f64 = msg_send![window, backingScaleFactor];
  if scale > 0.0 { scale } else { 1.0 }
}

unsafe fn logical_to_physical_size(parent_view: id, width: f64, height: f64) -> (u32, u32) {
  let scale = view_contents_scale(parent_view);
  let px_w = (width.max(1.0) * scale).round() as u32;
  let px_h = (height.max(1.0) * scale).round() as u32;
  (px_w.max(1), px_h.max(1))
}

unsafe fn create_metal_layer(parent_view: id, frame: NSRect) -> id {
  let Some(metal_layer_class) = Class::get("CAMetalLayer") else {
    eprintln!("[native-ui] CAMetalLayer class is unavailable");
    return nil;
  };
  let metal_layer: id = msg_send![metal_layer_class, layer];
  let scale = view_contents_scale(parent_view);
  let _: () = msg_send![metal_layer, setFrame: frame];
  let _: () = msg_send![metal_layer, setContentsScale: scale];
  let _: () = msg_send![metal_layer, setOpaque: false];
  let ns_color: id = msg_send![class!(NSColor), clearColor];
  let cg_color: *mut Object = msg_send![ns_color, CGColor];
  let _: () = msg_send![metal_layer, setBackgroundColor: cg_color];
  metal_layer
}

fn start_renderer(view_ptr: *mut Object, width: u32, height: u32) {
  stop_renderer();

  let running = Arc::new(AtomicBool::new(true));
  let pending_size = Arc::new(Mutex::new((width.max(1), height.max(1))));
  let running_for_thread = Arc::clone(&running);
  let size_for_thread = Arc::clone(&pending_size);

  let view_addr = view_ptr as usize;
  let thread = thread::spawn(move || {
    let view_ptr = view_addr as *mut c_void;
    let Some(view_non_null) = NonNull::<c_void>::new(view_ptr) else {
      eprintln!("[native-ui] invalid NSView pointer");
      return;
    };

    let raw_window_handle = RawWindowHandle::AppKit(AppKitWindowHandle::new(view_non_null));
    let raw_display_handle = RawDisplayHandle::AppKit(AppKitDisplayHandle::new());

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let surface = unsafe {
      instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle,
        raw_window_handle,
      })
    };

    let Ok(surface) = surface else {
      eprintln!("[native-ui] failed to create wgpu surface");
      return;
    };

    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
      power_preference: wgpu::PowerPreference::HighPerformance,
      compatible_surface: Some(&surface),
      force_fallback_adapter: false,
    })) {
      Some(adapter) => adapter,
      None => {
        eprintln!("[native-ui] no suitable wgpu adapter");
        return;
      }
    };

    let (device, queue) = match pollster::block_on(adapter.request_device(
      &wgpu::DeviceDescriptor {
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        label: Some("sujay-native-ui-device"),
      },
      None,
    )) {
      Ok(pair) => pair,
      Err(err) => {
        eprintln!("[native-ui] request_device failed: {err}");
        return;
      }
    };

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("sujay-native-ui-waveform-shader"),
      source: wgpu::ShaderSource::Wgsl(WAVEFORM_SHADER.into()),
    });

    let initial_size = *size_for_thread.lock().unwrap();
    let mut config = choose_surface_config(&surface, &adapter, initial_size.0, initial_size.1);
    surface.configure(&device, &config);
    let RendererResources {
      compute_bind_group_layout,
      compute_pipeline,
      render_pipeline,
      render_params_buffer,
      mut deck_states,
      render_bind_group,
    } = create_renderer_resources(&device, &shader, config.format);

    let mut last_versions = [0_u64, 0_u64];
    let mut frame_counter: f32 = 0.0;

    while running_for_thread.load(Ordering::Relaxed) {
      let current_versions = [
        WAVEFORM_VERSIONS[0].load(Ordering::Relaxed),
        WAVEFORM_VERSIONS[1].load(Ordering::Relaxed),
      ];
      sync_deck_waveforms(
        &device,
        &queue,
        &compute_bind_group_layout,
        &mut deck_states,
        current_versions,
        &mut last_versions,
        |deck_index| {
          let guard = WAVEFORMS.lock().unwrap();
          guard[deck_index].clone()
        },
      );

      let latest_size = *size_for_thread.lock().unwrap();
      if latest_size.0 != config.width || latest_size.1 != config.height {
        config.width = latest_size.0.max(1);
        config.height = latest_size.1.max(1);
        surface.configure(&device, &config);
      }

      let frame = match surface.get_current_texture() {
        Ok(frame) => frame,
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
          surface.configure(&device, &config);
          continue;
        }
        Err(wgpu::SurfaceError::Timeout) => {
          thread::sleep(Duration::from_millis(5));
          continue;
        }
        Err(wgpu::SurfaceError::OutOfMemory) => {
          eprintln!("[native-ui] wgpu surface out of memory");
          break;
        }
      };

      let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

      write_compute_params(&queue, &deck_states);

      let render_params = encode_u32_f32_f32_f32(
        PEAK_BINS,
        config.width as f32,
        config.height as f32,
        frame_counter,
      );
      queue.write_buffer(&render_params_buffer, 0, &render_params);

      let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sujay-native-ui-encoder"),
      });

      {
        let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
          label: Some("sujay-native-ui-peaks-pass"),
          timestamp_writes: None,
        });
        compute_pass.set_pipeline(&compute_pipeline);
        for deck in deck_states.iter() {
          compute_pass.set_bind_group(0, &deck.compute_bind_group, &[]);
          compute_pass.dispatch_workgroups((PEAK_BINS + COMPUTE_WORKGROUP_SIZE - 1) / COMPUTE_WORKGROUP_SIZE, 1, 1);
        }
      }

      {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
          label: Some("sujay-native-ui-waveform-pass"),
          color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
              load: wgpu::LoadOp::Clear(wgpu::Color {
                r: 0.06,
                g: 0.08,
                b: 0.12,
                a: 1.0,
              }),
              store: wgpu::StoreOp::Store,
            },
          })],
          depth_stencil_attachment: None,
          timestamp_writes: None,
          occlusion_query_set: None,
        });
        pass.set_pipeline(&render_pipeline);
        pass.set_bind_group(0, &render_bind_group, &[]);
        pass.draw(0..6, 0..1);
      }

      queue.submit(Some(encoder.finish()));
      frame.present();

      frame_counter += 1.0;

      thread::sleep(Duration::from_millis(16));
    }

  });

  *RENDERER.lock().unwrap() = Some(RendererState {
    running,
    pending_size,
    thread: Some(thread),
  });
}

fn resize_renderer(width: u32, height: u32) {
  let guard = RENDERER.lock().unwrap();
  if let Some(state) = guard.as_ref() {
    *state.pending_size.lock().unwrap() = (width.max(1), height.max(1));
  }
}

fn stop_renderer() {
  let mut guard = RENDERER.lock().unwrap();
  if let Some(mut state) = guard.take() {
    state.running.store(false, Ordering::Relaxed);
    if let Some(thread) = state.thread.take() {
      let _ = thread.join();
    }
  }
}

pub fn set_waveform(deck_index: usize, samples: Vec<f32>) {
  if deck_index > 1 {
    return;
  }

  let mut guard = WAVEFORMS.lock().unwrap();
  guard[deck_index] = samples;
  WAVEFORM_VERSIONS[deck_index].fetch_add(1, Ordering::Relaxed);
}

pub fn set_deck_progress(deck_index: usize, progress: f32, duration: f32) {
  if deck_index > 1 {
    return;
  }
  let mut guard = DECK_VISUALS.lock().unwrap();
  guard[deck_index].progress = progress.clamp(0.0, 1.0);
  guard[deck_index].duration = if duration.is_finite() && duration > 0.0 { duration } else { 0.0 };
}

pub fn set_deck_markers(deck_index: usize, beats: Vec<f32>, intro: Option<f32>, outro: Option<f32>) {
  if deck_index > 1 {
    return;
  }

  let mut guard = DECK_VISUALS.lock().unwrap();
  guard[deck_index].beats = beats
    .into_iter()
    .filter(|v| v.is_finite())
    .map(|v| v.clamp(0.0, 1.0))
    .collect();
  guard[deck_index].intro = intro.map(|v| v.clamp(0.0, 1.0));
  guard[deck_index].outro = outro.map(|v| v.clamp(0.0, 1.0));
}

pub fn set_console_state(state: ConsoleVisualState) {
  let mut guard = CONSOLE_VISUAL.lock().unwrap();
  *guard = state;
}

pub unsafe fn attach(parent_ptr: *mut std::ffi::c_void, x: f64, y: f64, width: f64, height: f64) {
  attach_to_nsview(parent_ptr, x, y, width, height);
}

/// Attach a colored native NSView as a subview of the Electron window's content view.
pub unsafe fn attach_to_nsview(parent_ptr: *mut std::ffi::c_void, x: f64, y: f64, width: f64, height: f64) {
  let parent_view = parent_ptr as id;
  if parent_view == nil {
    return;
  }

  if let Some(existing) = CHILD_VIEW.lock().unwrap().take() {
    let _: () = msg_send![existing.0, removeFromSuperview];
  }

  let frame = html_frame_to_nsview_frame(parent_view, x, y, width, height);
  let view: id = msg_send![class!(NSView), alloc];
  let view: id = msg_send![view, initWithFrame: frame];

  // Make it layer-backed and bind CAMetalLayer.
  let _: () = msg_send![view, setWantsLayer: true];
  let metal_layer = create_metal_layer(parent_view, frame);
  if metal_layer == nil {
    return;
  }
  let _: () = msg_send![view, setLayer: metal_layer];

  // Add as subview of the Electron content view
  // Keep native view above WebContents so it is always visible during migration.
  let _: () = msg_send![parent_view, addSubview: view positioned: 1_i64 relativeTo: nil];

  let (px_w, px_h) = logical_to_physical_size(parent_view, width, height);
  start_renderer(view as *mut Object, px_w, px_h);

  *CHILD_VIEW.lock().unwrap() = Some(ViewPtr(view));
}

/// Update the frame of the attached view.
pub unsafe fn set_frame(x: f64, y: f64, width: f64, height: f64) {
  let guard = CHILD_VIEW.lock().unwrap();
  if let Some(ref view_ptr) = *guard {
    let parent_view: id = msg_send![view_ptr.0, superview];
    if parent_view == nil {
      return;
    }
    let frame = html_frame_to_nsview_frame(parent_view, x, y, width, height);
    let _: () = msg_send![view_ptr.0, setFrame: frame];

    let layer: id = msg_send![view_ptr.0, layer];
    if layer != nil {
      let scale = view_contents_scale(parent_view);
      let _: () = msg_send![layer, setFrame: frame];
      let _: () = msg_send![layer, setContentsScale: scale];
    }

    let (px_w, px_h) = logical_to_physical_size(parent_view, width, height);
    resize_renderer(px_w, px_h);
  }
}

/// Remove the attached view.
pub unsafe fn detach() {
  stop_renderer();

  let mut guard = CHILD_VIEW.lock().unwrap();
  if let Some(view_ptr) = guard.take() {
    let _: () = msg_send![view_ptr.0, removeFromSuperview];
  }
}
