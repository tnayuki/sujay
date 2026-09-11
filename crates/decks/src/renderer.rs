//! macOS host-window glue for the shared egui DJ console.
//!
//! The immediate-mode UI, shared state, public setters and the render loop all
//! live in [`crate::console_ui`]. This file only owns the AppKit side: a custom
//! `NSView` subclass that forwards mouse + drag events, CAMetalLayer setup, the
//! Metal-backed wgpu surface, and the `attach` / `set_frame` / `detach` API.

// The `objc` crate's sel_impl!/class!/msg_send! macros internally emit
// references that trip newer lints; allow them crate-locally.
//
// The `cocoa` crate is deprecated in favour of objc2 but migration is a
// separate effort.
#![allow(unexpected_cfgs)]

use cocoa::base::{id, nil};
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use egui_wgpu::wgpu;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{
  AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

// Re-export the shared public UI API so `lib.rs` can keep calling `renderer::*`.
pub use crate::console_ui::*;
// Internals the AppKit event handlers feed into.
use crate::console_ui::{MouseEvent, MOUSE_EVENTS, NEEDS_REPAINT};

struct ViewPtr(*mut Object);
unsafe impl Send for ViewPtr {}
unsafe impl Sync for ViewPtr {}

struct RendererState {
  running: Arc<AtomicBool>,
  pending_size: Arc<Mutex<(u32, u32, f32)>>, // (px_w, px_h, scale)
  thread: Option<JoinHandle<()>>,
}

static CHILD_VIEW: Mutex<Option<ViewPtr>> = Mutex::new(None);
static RENDERER: Mutex<Option<RendererState>> = Mutex::new(None);

// ── Custom NSView subclass for mouse events ────────────────────────────────
use std::sync::Once;

static REGISTER_CLASS: Once = Once::new();

fn mouse_view_class() -> &'static Class {
  REGISTER_CLASS.call_once(|| {
    let superclass = class!(NSView);
    let mut decl = ClassDecl::new("SujayMouseView", superclass).unwrap();

    extern "C" fn accepts_first_responder(_this: &Object, _sel: Sel) -> bool {
      true
    }

    // Override hitTest: claim hits inside our bounds, but pass through
    // the traffic-light button area so Close/Minimize/Maximize still work.
    // `point` here is in this view's local coordinate system.
    extern "C" fn hit_test(this: &Object, _sel: Sel, point: NSPoint) -> id {
      unsafe {
        let bounds: NSRect = msg_send![this, bounds];
        let inside = point.x >= 0.0
          && point.x <= bounds.size.width
          && point.y >= 0.0
          && point.y <= bounds.size.height;
        if inside {
          // NSView uses bottom-left origin; top-left traffic lights
          // correspond to y in (h-38 .. h).
          if point.x < 80.0 && point.y > bounds.size.height - 38.0 {
            return nil; // let native traffic-light buttons handle it
          }
          this as *const Object as id
        } else {
          nil
        }
      }
    }

    extern "C" fn mouse_down(this: &Object, _sel: Sel, event: id) {
      let pt = local_point(this, event);
      MOUSE_EVENTS
        .lock()
        .unwrap()
        .push(MouseEvent::PressedPrimary(pt.0, pt.1));
      NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }
    extern "C" fn mouse_up(this: &Object, _sel: Sel, event: id) {
      let pt = local_point(this, event);
      MOUSE_EVENTS
        .lock()
        .unwrap()
        .push(MouseEvent::ReleasedPrimary(pt.0, pt.1));
      NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }
    extern "C" fn right_mouse_down(this: &Object, _sel: Sel, event: id) {
      let pt = local_point(this, event);
      MOUSE_EVENTS
        .lock()
        .unwrap()
        .push(MouseEvent::PressedSecondary(pt.0, pt.1));
      NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }
    extern "C" fn right_mouse_up(this: &Object, _sel: Sel, event: id) {
      let pt = local_point(this, event);
      MOUSE_EVENTS
        .lock()
        .unwrap()
        .push(MouseEvent::ReleasedSecondary(pt.0, pt.1));
      NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }
    extern "C" fn scroll_wheel(_this: &Object, _sel: Sel, event: id) {
      unsafe {
        let delta_y: f64 = msg_send![event, scrollingDeltaY];
        MOUSE_EVENTS
          .lock()
          .unwrap()
          .push(MouseEvent::Wheel(delta_y as f32));
        NEEDS_REPAINT.store(true, Ordering::Relaxed);
      }
    }
    extern "C" fn mouse_moved(this: &Object, _sel: Sel, event: id) {
      let pt = local_point(this, event);
      MOUSE_EVENTS
        .lock()
        .unwrap()
        .push(MouseEvent::Moved(pt.0, pt.1));
      NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }
    extern "C" fn mouse_dragged(this: &Object, _sel: Sel, event: id) {
      let pt = local_point(this, event);
      MOUSE_EVENTS
        .lock()
        .unwrap()
        .push(MouseEvent::Moved(pt.0, pt.1));
      NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }

    fn local_point(this: &Object, event: id) -> (f32, f32) {
      unsafe {
        let loc: NSPoint = msg_send![event, locationInWindow];
        let local: NSPoint = msg_send![this, convertPoint:loc fromView:nil];
        let bounds: NSRect = msg_send![this, bounds];
        // Flip Y (NSView origin is bottom-left, egui is top-left)
        // Do NOT multiply by scale — egui works in logical points
        (local.x as f32, (bounds.size.height - local.y) as f32)
      }
    }

    unsafe {
      decl.add_method(
        sel!(acceptsFirstResponder),
        accepts_first_responder as extern "C" fn(&Object, Sel) -> bool,
      );
      decl.add_method(
        sel!(hitTest:),
        hit_test as extern "C" fn(&Object, Sel, NSPoint) -> id,
      );
      decl.add_method(
        sel!(mouseDown:),
        mouse_down as extern "C" fn(&Object, Sel, id),
      );
      decl.add_method(sel!(mouseUp:), mouse_up as extern "C" fn(&Object, Sel, id));
      decl.add_method(
        sel!(rightMouseDown:),
        right_mouse_down as extern "C" fn(&Object, Sel, id),
      );
      decl.add_method(
        sel!(rightMouseUp:),
        right_mouse_up as extern "C" fn(&Object, Sel, id),
      );
      decl.add_method(
        sel!(mouseMoved:),
        mouse_moved as extern "C" fn(&Object, Sel, id),
      );
      decl.add_method(
        sel!(mouseDragged:),
        mouse_dragged as extern "C" fn(&Object, Sel, id),
      );
      decl.add_method(
        sel!(scrollWheel:),
        scroll_wheel as extern "C" fn(&Object, Sel, id),
      );
    }

    decl.register();
  });

  Class::get("SujayMouseView").unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════
// Render thread bootstrap (Metal surface → shared egui loop)
// ═══════════════════════════════════════════════════════════════════════════

fn start_renderer(view_ptr: *mut Object, width: u32, height: u32, scale: f32) {
  stop_renderer();

  // ── Create surface, adapter, device on MAIN THREAD (wgpu 24 requires it) ──
  let view_non_null = match NonNull::<c_void>::new(view_ptr as *mut c_void) {
    Some(v) => v,
    None => {
      eprintln!("[native-ui] invalid NSView pointer");
      return;
    }
  };

  let raw_window = RawWindowHandle::AppKit(AppKitWindowHandle::new(view_non_null));
  let raw_display = RawDisplayHandle::AppKit(AppKitDisplayHandle::new());

  let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
    backends: wgpu::Backends::METAL,
    ..Default::default()
  });
  let surface = unsafe {
    instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
      raw_display_handle: raw_display,
      raw_window_handle: raw_window,
    })
  };
  let surface = match surface {
    Ok(s) => s,
    Err(e) => {
      eprintln!("[native-ui] failed to create wgpu surface: {e}");
      return;
    }
  };

  let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
    power_preference: wgpu::PowerPreference::HighPerformance,
    compatible_surface: Some(&surface),
    force_fallback_adapter: false,
  })) {
    Some(a) => a,
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
      label: Some("sujay-egui"),
    },
    None,
  )) {
    Ok(pair) => pair,
    Err(err) => {
      eprintln!("[native-ui] request_device failed: {err}");
      return;
    }
  };

  let init_w = width.max(1);
  let init_h = height.max(1);
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
  let config = wgpu::SurfaceConfiguration {
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    format,
    width: init_w,
    height: init_h,
    present_mode,
    alpha_mode: caps.alpha_modes[0],
    view_formats: vec![],
    desired_maximum_frame_latency: 2,
  };
  surface.configure(&device, &config);

  // ── Move to render thread ──
  let running = Arc::new(AtomicBool::new(true));
  let pending_size = Arc::new(Mutex::new((init_w, init_h, scale)));
  let running_clone = Arc::clone(&running);
  let size_clone = Arc::clone(&pending_size);

  let thread = thread::spawn(move || {
    crate::console_ui::run_egui_render_loop(
      device,
      queue,
      surface,
      format,
      config,
      running_clone,
      size_clone,
    );
  });

  *RENDERER.lock().unwrap() = Some(RendererState {
    running,
    pending_size,
    thread: Some(thread),
  });
}

fn resize_renderer(width: u32, height: u32, scale: f32) {
  let guard = RENDERER.lock().unwrap();
  if let Some(state) = guard.as_ref() {
    *state.pending_size.lock().unwrap() = (width.max(1), height.max(1), scale);
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

// ═══════════════════════════════════════════════════════════════════════════
// macOS NSView management
// ═══════════════════════════════════════════════════════════════════════════

unsafe fn html_frame_to_nsview_frame(
  parent_view: id,
  x: f64,
  y: f64,
  width: f64,
  height: f64,
) -> NSRect {
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
  if scale > 0.0 {
    scale
  } else {
    1.0
  }
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

// ═══════════════════════════════════════════════════════════════════════════
// Public host-window API (called from lib.rs)
// ═══════════════════════════════════════════════════════════════════════════

pub unsafe fn attach(parent_ptr: *mut c_void, x: f64, y: f64, width: f64, height: f64) {
  let parent_view = parent_ptr as id;
  if parent_view == nil {
    return;
  }

  // Clean up previous view
  if let Some(existing) = CHILD_VIEW.lock().unwrap().take() {
    let _: () = msg_send![existing.0, removeFromSuperview];
  }

  let frame = html_frame_to_nsview_frame(parent_view, x, y, width, height);
  let cls = mouse_view_class();
  let view: id = msg_send![cls, alloc];
  let view: id = msg_send![view, initWithFrame: frame];

  // Keep drop handling on the host window (winit DroppedFile) to avoid
  // AppKit drag-session edge cases in this child NSView.

  // Add tracking area for mouseMoved events
  let tracking_options: u64 = 0x02 /* NSTrackingMouseMoved */
        | 0x20 /* NSTrackingActiveAlways */
        | 0x08 /* NSTrackingInVisibleRect */;
  let tracking_area: id = msg_send![class!(NSTrackingArea), alloc];
  let tracking_area: id = msg_send![tracking_area,
      initWithRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0))
      options: tracking_options
      owner: view
      userInfo: nil
  ];
  let _: () = msg_send![view, addTrackingArea: tracking_area];

  // Make it layer-backed — wgpu requires a CAMetalLayer on the view
  let _: () = msg_send![view, setWantsLayer: true];
  let metal_layer = create_metal_layer(parent_view, frame);
  if metal_layer == nil {
    return;
  }
  let _: () = msg_send![view, setLayer: metal_layer];

  // Add as subview above WebContents
  let _: () = msg_send![parent_view, addSubview: view positioned: 1_i64 relativeTo: nil];

  let (px_w, px_h) = logical_to_physical_size(parent_view, width, height);
  let scale = view_contents_scale(parent_view) as f32;

  start_renderer(view as *mut Object, px_w, px_h, scale);

  *CHILD_VIEW.lock().unwrap() = Some(ViewPtr(view));
}

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
    let scale = view_contents_scale(parent_view) as f32;
    resize_renderer(px_w, px_h, scale);
  }
}

pub unsafe fn detach() {
  stop_renderer();
  let mut guard = CHILD_VIEW.lock().unwrap();
  if let Some(view_ptr) = guard.take() {
    let _: () = msg_send![view_ptr.0, removeFromSuperview];
  }
}
