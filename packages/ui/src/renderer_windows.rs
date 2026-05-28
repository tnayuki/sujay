use crate::ui_state::ConsoleVisualState;
use crate::renderer_wgpu_shared::{
  choose_surface_config, create_renderer_resources, encode_u32_f32_f32_f32,
  sync_deck_waveforms, write_compute_params, RendererResources, COMPUTE_WORKGROUP_SIZE,
  PEAK_BINS, WAVEFORM_SHADER,
};
use raw_window_handle::{
  RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::ffi::c_void;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
  CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, SW_SHOW, WS_CHILD, WS_VISIBLE,
};

struct RendererState {
  running: Arc<AtomicBool>,
  pending_size: Arc<Mutex<(u32, u32)>>,
  thread: Option<JoinHandle<()>>,
}

static RENDERER: Mutex<Option<RendererState>> = Mutex::new(None);
static CHILD_HWND: Mutex<Option<usize>> = Mutex::new(None);
static WAVEFORMS: Mutex<[Vec<f32>; 2]> = Mutex::new([Vec::new(), Vec::new()]);
static CONSOLE_VISUAL: Mutex<ConsoleVisualState> = Mutex::new(ConsoleVisualState {
  deck_a: crate::ui_state::DeckConsoleVisualState {
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
  deck_b: crate::ui_state::DeckConsoleVisualState {
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

fn f64_to_i32(value: f64, fallback: i32) -> i32 {
  if !value.is_finite() {
    return fallback;
  }
  value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

fn to_wide_null(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn create_child_window(parent: HWND, x: i32, y: i32, width: i32, height: i32) -> Option<HWND> {
  let class_name = to_wide_null("STATIC");
  let title = to_wide_null("");
  let hinstance = GetModuleHandleW(std::ptr::null());
  let child = CreateWindowExW(
    0,
    class_name.as_ptr(),
    title.as_ptr(),
    WS_CHILD | WS_VISIBLE,
    x,
    y,
    width.max(1),
    height.max(1),
    parent,
    std::ptr::null_mut(),
    hinstance,
    std::ptr::null_mut(),
  );

  if child.is_null() {
    None
  } else {
    let _ = ShowWindow(child, SW_SHOW);
    Some(child)
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

fn resize_renderer(width: u32, height: u32) {
  let guard = RENDERER.lock().unwrap();
  if let Some(state) = guard.as_ref() {
    *state.pending_size.lock().unwrap() = (width.max(1), height.max(1));
  }
}

fn start_renderer(hwnd_ptr: *mut c_void, width: u32, height: u32) {
  stop_renderer();

  let hwnd_addr = hwnd_ptr as usize;
  let running = Arc::new(AtomicBool::new(true));
  let pending_size = Arc::new(Mutex::new((width.max(1), height.max(1))));
  let running_for_thread = Arc::clone(&running);
  let size_for_thread = Arc::clone(&pending_size);

  let thread = thread::spawn(move || {
    let hwnd = hwnd_addr as isize;
    let Some(hwnd_nz) = NonZeroIsize::new(hwnd) else {
      eprintln!("[native-ui/windows] invalid HWND");
      return;
    };

    let raw_window_handle = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd_nz));
    let raw_display_handle = RawDisplayHandle::Windows(WindowsDisplayHandle::new());

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let surface = unsafe {
      instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle,
        raw_window_handle,
      })
    };

    let Ok(surface) = surface else {
      eprintln!("[native-ui/windows] failed to create wgpu surface");
      return;
    };

    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
      power_preference: wgpu::PowerPreference::HighPerformance,
      compatible_surface: Some(&surface),
      force_fallback_adapter: false,
    })) {
      Some(adapter) => adapter,
      None => {
        eprintln!("[native-ui/windows] no suitable wgpu adapter");
        return;
      }
    };

    let (device, queue) = match pollster::block_on(adapter.request_device(
      &wgpu::DeviceDescriptor {
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        label: Some("sujay-native-ui-win-device"),
      },
      None,
    )) {
      Ok(pair) => pair,
      Err(err) => {
        eprintln!("[native-ui/windows] request_device failed: {err}");
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
    let mut frame_counter = 0.0_f32;

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
          eprintln!("[native-ui/windows] wgpu surface out of memory");
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
          compute_pass.dispatch_workgroups(
            (PEAK_BINS + COMPUTE_WORKGROUP_SIZE - 1) / COMPUTE_WORKGROUP_SIZE,
            1,
            1,
          );
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

pub unsafe fn attach(parent_ptr: *mut std::ffi::c_void, _x: f64, _y: f64, width: f64, height: f64) {
  detach();

  let parent_hwnd = parent_ptr as HWND;
  if parent_hwnd.is_null() {
    return;
  }

  let x = f64_to_i32(_x, 0);
  let y = f64_to_i32(_y, 0);
  let w_i32 = f64_to_i32(width.max(1.0), 1).max(1);
  let h_i32 = f64_to_i32(height.max(1.0), 1).max(1);

  let target_hwnd = match create_child_window(parent_hwnd, x, y, w_i32, h_i32) {
    Some(child) => {
      *CHILD_HWND.lock().unwrap() = Some(child as usize);
      child
    }
    None => {
      eprintln!("[native-ui/windows] failed to create child window, fallback to parent HWND");
      parent_hwnd
    }
  };

  start_renderer(target_hwnd as *mut c_void, w_i32 as u32, h_i32 as u32);
}

pub unsafe fn set_frame(_x: f64, _y: f64, width: f64, height: f64) {
  let x = f64_to_i32(_x, 0);
  let y = f64_to_i32(_y, 0);
  let w_i32 = f64_to_i32(width.max(1.0), 1).max(1);
  let h_i32 = f64_to_i32(height.max(1.0), 1).max(1);

  if let Some(child) = *CHILD_HWND.lock().unwrap() {
    let hwnd = child as HWND;
    let _ = MoveWindow(hwnd, x, y, w_i32, h_i32, 1);
    let _ = ShowWindow(hwnd, SW_SHOW);
  }

  resize_renderer(w_i32 as u32, h_i32 as u32);
}

pub unsafe fn detach() {
  stop_renderer();

  if let Some(child) = CHILD_HWND.lock().unwrap().take() {
    let _ = DestroyWindow(child as HWND);
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

pub fn set_deck_progress(_deck_index: usize, _progress: f32, _duration: f32) {}

pub fn set_deck_markers(_deck_index: usize, _beats: Vec<f32>, _intro: Option<f32>, _outro: Option<f32>) {}

pub fn set_console_state(state: ConsoleVisualState) {
  let mut guard = CONSOLE_VISUAL.lock().unwrap();
  *guard = state;
}
