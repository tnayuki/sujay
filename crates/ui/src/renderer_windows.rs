//! Windows host-window glue for the shared egui DJ console.
//!
//! The immediate-mode UI, shared state, public setters and the render loop all
//! live in [`crate::console_ui`]. This file only owns the Win32 side: a child
//! `HWND` hosted inside the parent window, a DX12-backed wgpu surface, and the
//! `attach` / `set_frame` / `detach` API.

use egui_wgpu::wgpu;
use raw_window_handle::{
  RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::ffi::c_void;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
  CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, SW_SHOW, WS_CHILD, WS_VISIBLE,
};

// Re-export the shared public UI API so `lib.rs` can keep calling `renderer::*`.
pub use crate::console_ui::*;

struct RendererState {
  running: Arc<AtomicBool>,
  pending_size: Arc<Mutex<(u32, u32, f32)>>, // (px_w, px_h, scale)
  thread: Option<JoinHandle<()>>,
}

static RENDERER: Mutex<Option<RendererState>> = Mutex::new(None);
static CHILD_HWND: Mutex<Option<usize>> = Mutex::new(None);

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
    let mut size = state.pending_size.lock().unwrap();
    *size = (width.max(1), height.max(1), size.2);
  }
}

fn start_renderer(hwnd_ptr: *mut c_void, width: u32, height: u32) {
  stop_renderer();

  let hwnd_addr = hwnd_ptr as usize;
  let init_w = width.max(1);
  let init_h = height.max(1);
  // TODO: derive pixels_per_point from GetDpiForWindow for HiDPI displays.
  let scale = 1.0_f32;
  let running = Arc::new(AtomicBool::new(true));
  let pending_size = Arc::new(Mutex::new((init_w, init_h, scale)));
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

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
      backends: wgpu::Backends::DX12,
      ..Default::default()
    });
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

    // Request exactly what the adapter supports. egui needs only a small
    // subset, and the DX12→vkd3d→MoltenVK chain under Wine reports lower
    // limits than `Limits::default()` (e.g. max_storage_textures = 2), which
    // would otherwise fail device creation.
    let (device, queue) = match pollster::block_on(adapter.request_device(
      &wgpu::DeviceDescriptor {
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        memory_hints: wgpu::MemoryHints::Performance,
        label: Some("sujay-egui-win-device"),
      },
      None,
    )) {
      Ok(pair) => pair,
      Err(err) => {
        eprintln!("[native-ui/windows] request_device failed: {err}");
        return;
      }
    };

    let caps = surface.get_capabilities(&adapter);
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
    let init_size = *size_for_thread.lock().unwrap();
    let config = wgpu::SurfaceConfiguration {
      usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
      format,
      width: init_size.0.max(1),
      height: init_size.1.max(1),
      present_mode,
      alpha_mode: caps.alpha_modes[0],
      view_formats: vec![],
      desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    crate::console_ui::run_egui_render_loop(
      device, queue, surface, format, config, running_for_thread, size_for_thread,
    );
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
