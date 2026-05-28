use cocoa::base::{id, nil};
use cocoa::foundation::{NSPoint, NSRect, NSSize};
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
static WAVEFORM_CONTAINER_LAYERS: Mutex<[Option<ViewPtr>; 2]> = Mutex::new([None, None]);
static DECOR_LAYER: Mutex<Option<ViewPtr>> = Mutex::new(None);

#[derive(Clone, Default)]
struct DeckVisualState {
  progress: f32,
  duration: f32,
  beats: Vec<f32>,
  intro: Option<f32>,
  outro: Option<f32>,
}

const PEAK_BINS: u32 = 1024;
const COMPUTE_WORKGROUP_SIZE: u32 = 64;
const WAVEFORM_SHADER: &str = include_str!("waveform.wgsl");

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

struct DeckGpuState {
  sample_buffer: wgpu::Buffer,
  peak_buffer: wgpu::Buffer,
  compute_params_buffer: wgpu::Buffer,
  compute_bind_group: wgpu::BindGroup,
  sample_count: u32,
}

fn encode_u32_f32_f32_f32(a: u32, b: f32, c: f32, d: f32) -> [u8; 16] {
  let mut out = [0u8; 16];
  out[0..4].copy_from_slice(&a.to_ne_bytes());
  out[4..8].copy_from_slice(&b.to_bits().to_ne_bytes());
  out[8..12].copy_from_slice(&c.to_bits().to_ne_bytes());
  out[12..16].copy_from_slice(&d.to_bits().to_ne_bytes());
  out
}

fn encode_u32x4(a: u32, b: u32, c: u32, d: u32) -> [u8; 16] {
  let mut out = [0u8; 16];
  out[0..4].copy_from_slice(&a.to_ne_bytes());
  out[4..8].copy_from_slice(&b.to_ne_bytes());
  out[8..12].copy_from_slice(&c.to_ne_bytes());
  out[12..16].copy_from_slice(&d.to_ne_bytes());
  out
}

unsafe fn create_waveform_container_layer(parent_layer: id, frame: NSRect) -> id {
  let layer: id = msg_send![class!(CALayer), layer];
  let _: () = msg_send![layer, setFrame: frame];
  let _: () = msg_send![layer, setOpaque: false];
  let clear: id = msg_send![class!(NSColor), clearColor];
  let clear_cg: *mut Object = msg_send![clear, CGColor];
  let _: () = msg_send![layer, setBackgroundColor: clear_cg];
  let _: () = msg_send![parent_layer, addSublayer: layer];
  layer
}

unsafe fn clear_sublayers(layer: id) {
  let sublayers: id = msg_send![layer, sublayers];
  if sublayers == nil {
    return;
  }
  let count: usize = msg_send![sublayers, count];
  for i in (0..count).rev() {
    let sub: id = msg_send![sublayers, objectAtIndex: i];
    if sub != nil {
      let _: () = msg_send![sub, removeFromSuperlayer];
    }
  }
}

unsafe fn make_ns_color(red: f64, green: f64, blue: f64, alpha: f64) -> *mut Object {
  let ns_color: id = msg_send![class!(NSColor), colorWithRed: red green: green blue: blue alpha: alpha];
  let cg_color: *mut Object = msg_send![ns_color, CGColor];
  cg_color
}

unsafe fn add_vertical_line(container: id, x: f64, line_width: f64, color: *mut Object, height: f64) {
  let line_layer: id = msg_send![class!(CALayer), layer];
  let frame = NSRect::new(
    NSPoint::new(x - line_width * 0.5, 0.0),
    NSSize::new(line_width.max(1.0), height),
  );
  let _: () = msg_send![line_layer, setFrame: frame];
  let _: () = msg_send![line_layer, setBackgroundColor: color];
  let _: () = msg_send![container, addSublayer: line_layer];
}

unsafe fn add_text_layer(container: id, text: &str, x: f64, y: f64, width: f64, height: f64, font_size: f64, color: *mut Object, _align: i64) {
  let text_layer: id = msg_send![class!(CATextLayer), layer];
  let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width.max(1.0), height.max(1.0)));
  let _: () = msg_send![text_layer, setFrame: frame];
  let safe = text.replace('\0', " ");
  let c_text = std::ffi::CString::new(safe).unwrap_or_else(|_| std::ffi::CString::new(" ").unwrap());
  let ns_string: id = msg_send![class!(NSString), stringWithUTF8String: c_text.as_ptr()];
  let _: () = msg_send![text_layer, setString: ns_string];
  let _: () = msg_send![text_layer, setForegroundColor: color];
  let _: () = msg_send![text_layer, setFontSize: font_size];
  let _: () = msg_send![text_layer, setContentsScale: 2.0_f64];
  let _: () = msg_send![container, addSublayer: text_layer];
}

unsafe fn draw_waveform_bars_rect(container: id, samples: &[f32], progress: f32, rect: NSRect) {
  let width = rect.size.width.max(1.0);
  let height = rect.size.height.max(1.0);
  let x0 = rect.origin.x;
  let y0 = rect.origin.y;

  if samples.is_empty() {
    let placeholder: id = msg_send![class!(CALayer), layer];
    let _: () = msg_send![placeholder, setFrame: rect];
    let c = make_ns_color(0.16, 0.16, 0.16, 1.0);
    let _: () = msg_send![placeholder, setBackgroundColor: c];
    let _: () = msg_send![container, addSublayer: placeholder];
    return;
  }

  let max_bars = 256usize;
  let bar_count = samples.len().min(max_bars).max(1);
  let step = (samples.len() as f64 / bar_count as f64).max(1.0);
  let bar_width = (width / bar_count as f64).max(1.0);

  let played_color = make_ns_color(0.290, 0.620, 1.0, 0.95);
  let unplayed_color = make_ns_color(0.867, 0.867, 0.867, 0.92);

  for i in 0..bar_count {
    let src_index = ((i as f64) * step).floor() as usize;
    let idx = src_index.min(samples.len() - 1);
    let amp = f64::from(samples[idx].abs()).clamp(0.0, 1.0);
    let bar_height = (amp * height * 0.92).max(1.0);
    let x = x0 + (i as f64) * bar_width;
    let y = y0 + (height - bar_height) * 0.5;

    let bar_layer: id = msg_send![class!(CALayer), layer];
    let frame = NSRect::new(
      NSPoint::new(x, y),
      NSSize::new((bar_width - 0.4).max(1.0), bar_height),
    );
    let _: () = msg_send![bar_layer, setFrame: frame];
    let ratio = ((i as f32) / (bar_count as f32)).clamp(0.0, 1.0);
    let color = if ratio <= progress { played_color } else { unplayed_color };
    let _: () = msg_send![bar_layer, setBackgroundColor: color];
    let _: () = msg_send![container, addSublayer: bar_layer];
  }

  let progress_x = x0 + f64::from(progress.clamp(0.0, 1.0)) * width;
  let playhead_color = make_ns_color(1.0, 1.0, 1.0, 1.0);
  add_vertical_line(container, progress_x, 2.0, playhead_color, height);
}

unsafe fn draw_control_button(container: id, x: f64, y: f64, w: f64, h: f64, active: bool, accent: bool) {
  let button: id = msg_send![class!(CALayer), layer];
  let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
  let _: () = msg_send![button, setFrame: frame];
  let base = if active {
    if accent { make_ns_color(1.0, 0.27, 0.0, 1.0) } else { make_ns_color(0.0, 0.8, 0.4, 1.0) }
  } else {
    make_ns_color(0.20, 0.20, 0.20, 1.0)
  };
  let _: () = msg_send![button, setBackgroundColor: base];
  let _: () = msg_send![container, addSublayer: button];
}

unsafe fn redraw_console_decor() {
  let view_ptr = {
    let guard = CHILD_VIEW.lock().unwrap();
    match guard.as_ref() {
      Some(v) => v.0 as id,
      None => return,
    }
  };

  let decor_ptr = {
    let guard = DECOR_LAYER.lock().unwrap();
    match guard.as_ref() {
      Some(v) => v.0 as id,
      None => return,
    }
  };

  clear_sublayers(decor_ptr);

  let bounds: NSRect = msg_send![view_ptr, bounds];
  let w = bounds.size.width.max(1.0);
  let h = bounds.size.height.max(1.0);
  let padding = 10.0;
  let top_wave_h = 80.0;
  let top_gap = 5.0;
  let top_total_h = top_wave_h * 2.0 + top_gap;
  let top_block_bottom = h - padding - top_total_h;
  let deck_gap = 8.0;
  let tempo_w = 110.0;
  let crossfader_h = 34.0;
  let crossfader_bottom = padding;
  let deck_bottom = crossfader_bottom + crossfader_h + 12.0;
  let deck_top = (top_block_bottom - 20.0).max(deck_bottom + 100.0);
  let deck_h = (deck_top - deck_bottom).max(100.0);
  let deck_w = ((w - padding * 2.0 - tempo_w - deck_gap * 2.0) * 0.5).max(120.0);
  let left_x = padding;
  let tempo_x = left_x + deck_w + deck_gap;
  let right_x = tempo_x + tempo_w + deck_gap;

  let bg: id = msg_send![class!(CALayer), layer];
  let _: () = msg_send![bg, setFrame: bounds];
  let _: () = msg_send![bg, setBackgroundColor: make_ns_color(0.08, 0.08, 0.08, 1.0)];
  let _: () = msg_send![decor_ptr, addSublayer: bg];

  let left_panel = NSRect::new(NSPoint::new(left_x, deck_bottom), NSSize::new(deck_w, deck_h));
  let right_panel = NSRect::new(NSPoint::new(right_x, deck_bottom), NSSize::new(deck_w, deck_h));
  let tempo_panel = NSRect::new(NSPoint::new(tempo_x, deck_bottom), NSSize::new(tempo_w, deck_h));

  for panel in [left_panel, right_panel, tempo_panel] {
    let layer: id = msg_send![class!(CALayer), layer];
    let _: () = msg_send![layer, setFrame: panel];
    let _: () = msg_send![layer, setBackgroundColor: make_ns_color(0.15, 0.15, 0.15, 0.95)];
    let _: () = msg_send![decor_ptr, addSublayer: layer];
  }

  let console = CONSOLE_VISUAL.lock().unwrap().clone();
  let waveforms = WAVEFORMS.lock().unwrap().clone();
  let deck_visuals = DECK_VISUALS.lock().unwrap().clone();

  let left_full = NSRect::new(
    NSPoint::new(left_x + 8.0, deck_top - 95.0),
    NSSize::new(deck_w - 16.0, 40.0),
  );
  let right_full = NSRect::new(
    NSPoint::new(right_x + 8.0, deck_top - 95.0),
    NSSize::new(deck_w - 16.0, 40.0),
  );

  draw_waveform_bars_rect(decor_ptr, &waveforms[0], deck_visuals[0].progress, left_full);
  draw_waveform_bars_rect(decor_ptr, &waveforms[1], deck_visuals[1].progress, right_full);

  let text_color = make_ns_color(0.88, 0.88, 0.88, 1.0);
  let bpm_color = make_ns_color(1.0, 0.42, 0.2, 1.0);
  let cyan = make_ns_color(0.0, 0.83, 1.0, 1.0);
  add_text_layer(decor_ptr, "1", left_x + 8.0, deck_top - 26.0, 22.0, 18.0, 17.0, cyan, 0);
  add_text_layer(decor_ptr, "2", right_x + deck_w - 24.0, deck_top - 26.0, 22.0, 18.0, 17.0, cyan, 2);
  add_text_layer(decor_ptr, &console.deck_a.title, left_x + 34.0, deck_top - 28.0, deck_w - 80.0, 16.0, 12.0, text_color, 0);
  add_text_layer(decor_ptr, &console.deck_b.title, right_x + 8.0, deck_top - 28.0, deck_w - 80.0, 16.0, 12.0, text_color, 0);
  add_text_layer(decor_ptr, &console.deck_a.time_text, left_x + 34.0, deck_top - 44.0, deck_w - 90.0, 14.0, 10.0, cyan, 0);
  add_text_layer(decor_ptr, &console.deck_b.time_text, right_x + 8.0, deck_top - 44.0, deck_w - 90.0, 14.0, 10.0, cyan, 0);
  add_text_layer(decor_ptr, &console.deck_a.bpm_text, left_x + deck_w - 56.0, deck_top - 44.0, 48.0, 14.0, 10.0, bpm_color, 2);
  add_text_layer(decor_ptr, &console.deck_b.bpm_text, right_x + deck_w - 56.0, deck_top - 44.0, 48.0, 14.0, 10.0, bpm_color, 2);

  draw_control_button(decor_ptr, left_x + 8.0, deck_top - 52.0, 24.0, 24.0, console.deck_a.playing, true);
  draw_control_button(decor_ptr, right_x + deck_w - 32.0, deck_top - 52.0, 24.0, 24.0, console.deck_b.playing, true);

  // Tempo display
  let tempo_text = format!("{:03}", console.master_tempo.round().clamp(0.0, 999.0) as i32);
  let tempo_bg = NSRect::new(
    NSPoint::new(tempo_x + 14.0, deck_top - 74.0),
    NSSize::new(tempo_w - 28.0, 40.0),
  );
  let tempo_layer: id = msg_send![class!(CALayer), layer];
  let _: () = msg_send![tempo_layer, setFrame: tempo_bg];
  let _: () = msg_send![tempo_layer, setBackgroundColor: make_ns_color(0.0, 0.0, 0.0, 1.0)];
  let _: () = msg_send![decor_ptr, addSublayer: tempo_layer];
  let orange = make_ns_color(1.0, 0.27, 0.0, 1.0);
  add_text_layer(decor_ptr, &tempo_text, tempo_x + 18.0, deck_top - 67.0, tempo_w - 36.0, 28.0, 24.0, orange, 1);

  // Level meters and EQ kill buttons
  let meter_base_y = deck_bottom + 20.0;
  let meter_h = (deck_h - 120.0).max(40.0);
  let left_meter_x = tempo_x + 20.0;
  let right_meter_x = tempo_x + tempo_w - 30.0;
  let left_level_h = meter_h * f64::from(console.deck_a.peak.clamp(0.0, 1.0));
  let right_level_h = meter_h * f64::from(console.deck_b.peak.clamp(0.0, 1.0));

  for (x, fill_h) in [(left_meter_x, left_level_h), (right_meter_x, right_level_h)] {
    let slot: id = msg_send![class!(CALayer), layer];
    let _: () = msg_send![slot, setFrame: NSRect::new(NSPoint::new(x, meter_base_y), NSSize::new(10.0, meter_h))];
    let _: () = msg_send![slot, setBackgroundColor: make_ns_color(0.06, 0.06, 0.06, 1.0)];
    let _: () = msg_send![decor_ptr, addSublayer: slot];

    let fill: id = msg_send![class!(CALayer), layer];
    let _: () = msg_send![fill, setFrame: NSRect::new(NSPoint::new(x, meter_base_y), NSSize::new(10.0, fill_h.max(1.0)))];
    let _: () = msg_send![fill, setBackgroundColor: make_ns_color(0.0, 0.85, 1.0, 0.95)];
    let _: () = msg_send![decor_ptr, addSublayer: fill];
  }

  let eq_left_x = tempo_x + 2.0;
  let eq_right_x = tempo_x + tempo_w - 18.0;
  let eq_y = meter_base_y + meter_h + 6.0;
  draw_control_button(decor_ptr, eq_left_x, eq_y, 14.0, 12.0, console.deck_a.eq_high, false);
  draw_control_button(decor_ptr, eq_left_x, eq_y + 15.0, 14.0, 12.0, console.deck_a.eq_mid, false);
  draw_control_button(decor_ptr, eq_left_x, eq_y + 30.0, 14.0, 12.0, console.deck_a.eq_low, false);
  draw_control_button(decor_ptr, eq_right_x, eq_y, 14.0, 12.0, console.deck_b.eq_high, false);
  draw_control_button(decor_ptr, eq_right_x, eq_y + 15.0, 14.0, 12.0, console.deck_b.eq_mid, false);
  draw_control_button(decor_ptr, eq_right_x, eq_y + 30.0, 14.0, 12.0, console.deck_b.eq_low, false);

  // Crossfader
  let track_y = crossfader_bottom + 12.0;
  let track_x = padding + 20.0;
  let track_w = (w - padding * 2.0 - 40.0).max(80.0);
  let track: id = msg_send![class!(CALayer), layer];
  let _: () = msg_send![track, setFrame: NSRect::new(NSPoint::new(track_x, track_y), NSSize::new(track_w, 6.0))];
  let _: () = msg_send![track, setBackgroundColor: make_ns_color(0.20, 0.20, 0.20, 1.0)];
  let _: () = msg_send![decor_ptr, addSublayer: track];

  let slider_x = track_x + f64::from(console.crossfader.clamp(0.0, 1.0)) * track_w;
  let slider: id = msg_send![class!(CALayer), layer];
  let _: () = msg_send![slider, setFrame: NSRect::new(NSPoint::new(slider_x - 5.0, track_y - 9.0), NSSize::new(10.0, 24.0))];
  let _: () = msg_send![slider, setBackgroundColor: make_ns_color(0.33, 0.33, 0.33, 1.0)];
  let _: () = msg_send![decor_ptr, addSublayer: slider];
}

unsafe fn redraw_deck_layer(deck_index: usize) {
  let container_ptr = {
    let layer_guard = WAVEFORM_CONTAINER_LAYERS.lock().unwrap();
    match layer_guard[deck_index].as_ref() {
      Some(layer) => layer.0,
      None => return,
    }
  };

  let samples = {
    let waveform_guard = WAVEFORMS.lock().unwrap();
    waveform_guard[deck_index].clone()
  };

  let visual = {
    let visual_guard = DECK_VISUALS.lock().unwrap();
    visual_guard[deck_index].clone()
  };

  clear_sublayers(container_ptr as id);

  if samples.is_empty() {
    return;
  }

  let bounds: NSRect = msg_send![container_ptr, bounds];
  let width = bounds.size.width.max(1.0);
  let height = bounds.size.height.max(1.0);

  let zoom_window_seconds = 8.0_f32;
  let playhead_offset = 0.5_f32;
  let window_ratio = if visual.duration.is_finite() && visual.duration > 0.0 {
    (zoom_window_seconds / visual.duration).max(0.0001)
  } else {
    1.0
  };
  let is_zoomed = visual.duration.is_finite() && visual.duration > 0.0;
  let viewport_span = if is_zoomed { window_ratio } else { 1.0 };
  let viewport_start = if is_zoomed {
    visual.progress - viewport_span * playhead_offset
  } else {
    0.0
  };
  let viewport_end = viewport_start + viewport_span;

  let max_bars = 512usize;
  let bar_count = samples.len().min(max_bars).max(1);
  let bar_width = (width / bar_count as f64).max(1.0);
  let progress_x = if is_zoomed {
    let visible_progress = ((visual.progress - viewport_start) / viewport_span).clamp(0.0, 1.0);
    f64::from(visible_progress) * width
  } else {
    f64::from(visual.progress.clamp(0.0, 1.0)) * width
  };

  let played_color = make_ns_color(0.290, 0.620, 1.0, 0.95);
  let unplayed_color = make_ns_color(0.867, 0.867, 0.867, 0.92);

  for i in 0..bar_count {
    let sample_ratio = viewport_start
      + (((i as f32) + 0.5) / (bar_count as f32)) * viewport_span;
    if !(0.0..=1.0).contains(&sample_ratio) {
      continue;
    }
    let src_index = (sample_ratio * ((samples.len() - 1) as f32)).round() as usize;
    let idx = src_index.min(samples.len() - 1);
    let amp = f64::from(samples[idx].abs()).clamp(0.0, 1.0);
    let bar_height = (amp * height * 0.90).max(1.0);
    let x = (i as f64) * bar_width;
    let y = (height - bar_height) * 0.5;

    let bar_layer: id = msg_send![class!(CALayer), layer];
    let frame = NSRect::new(
      NSPoint::new(x, y),
      NSSize::new((bar_width - 0.5).max(1.0), bar_height),
    );
    let _: () = msg_send![bar_layer, setFrame: frame];
    let color = if sample_ratio <= visual.progress { played_color } else { unplayed_color };
    let _: () = msg_send![bar_layer, setBackgroundColor: color];
    let _: () = msg_send![container_ptr, addSublayer: bar_layer];
  }

  // Beat markers (rgba(255,100,100,0.8))
  let beat_color = make_ns_color(1.0, 100.0 / 255.0, 100.0 / 255.0, 0.8);
  for beat in visual.beats.iter() {
    if *beat < viewport_start || *beat > viewport_end {
      continue;
    }
    let x = f64::from(((*beat - viewport_start) / viewport_span).clamp(0.0, 1.0)) * width;
    add_vertical_line(container_ptr as id, x, 1.0, beat_color, height);
  }

  // Intro marker (rgba(100,255,100,0.8))
  if let Some(intro) = visual.intro {
    if intro >= viewport_start && intro <= viewport_end {
      let x = f64::from(((intro - viewport_start) / viewport_span).clamp(0.0, 1.0)) * width;
      let intro_color = make_ns_color(100.0 / 255.0, 1.0, 100.0 / 255.0, 0.8);
      add_vertical_line(container_ptr as id, x, 2.0, intro_color, height);
    }
  }

  // Outro marker (rgba(255,255,100,0.8))
  if let Some(outro) = visual.outro {
    if outro >= viewport_start && outro <= viewport_end {
      let x = f64::from(((outro - viewport_start) / viewport_span).clamp(0.0, 1.0)) * width;
      let outro_color = make_ns_color(1.0, 1.0, 100.0 / 255.0, 0.8);
      add_vertical_line(container_ptr as id, x, 2.0, outro_color, height);
    }
  }

  // Playhead line (#ffffff)
  let playhead_color = make_ns_color(1.0, 1.0, 1.0, 1.0);
  add_vertical_line(container_ptr as id, progress_x, 2.0, playhead_color, height);
}

unsafe fn redraw_all_decks() {
  redraw_console_decor();
  redraw_deck_layer(0);
  redraw_deck_layer(1);
}

unsafe fn draw_waveform_bars(container: id, samples: &[f32], red: f64, green: f64, blue: f64) {
  clear_sublayers(container);

  if samples.is_empty() {
    return;
  }

  let bounds: NSRect = msg_send![container, bounds];
  let width = bounds.size.width.max(1.0);
  let height = bounds.size.height.max(1.0);

  let max_bars = 512usize;
  let bar_count = samples.len().min(max_bars).max(1);
  let step = (samples.len() as f64 / bar_count as f64).max(1.0);
  let bar_width = (width / bar_count as f64).max(1.0);

  let ns_color: id = msg_send![class!(NSColor), colorWithRed: red green: green blue: blue alpha: 0.95_f64];
  let cg_color: *mut Object = msg_send![ns_color, CGColor];

  for i in 0..bar_count {
    let src_index = ((i as f64) * step).floor() as usize;
    let idx = src_index.min(samples.len() - 1);
    let amp = f64::from(samples[idx].abs()).clamp(0.0, 1.0);
    let bar_height = (amp * height * 0.92).max(1.0);
    let x = (i as f64) * bar_width;
    let y = (height - bar_height) * 0.5;

    let bar_layer: id = msg_send![class!(CALayer), layer];
    let frame = NSRect::new(
      NSPoint::new(x, y),
      NSSize::new((bar_width - 0.5).max(1.0), bar_height),
    );
    let _: () = msg_send![bar_layer, setFrame: frame];
    let _: () = msg_send![bar_layer, setBackgroundColor: cg_color];
    let _: () = msg_send![container, addSublayer: bar_layer];
  }
}

unsafe fn layout_waveform_containers(view: id) {
  let layer: id = msg_send![view, layer];
  if layer == nil {
    return;
  }

  let bounds: NSRect = msg_send![view, bounds];
  let w = bounds.size.width.max(1.0);
  let h = bounds.size.height.max(1.0);
  let padding = 10.0;
  let deck_gap = 8.0;
  let top_wave_h = 80.0;
  let deck_w = ((w - padding * 2.0 - deck_gap) * 0.5).max(24.0);
  let top_wave_y = (h - padding - top_wave_h).max(0.0);

  let left_frame = NSRect::new(
    NSPoint::new(padding, top_wave_y),
    NSSize::new(deck_w, top_wave_h.max(1.0)),
  );
  let right_frame = NSRect::new(
    NSPoint::new(padding + deck_w + deck_gap, top_wave_y),
    NSSize::new(deck_w, top_wave_h.max(1.0)),
  );

  let mut guard = WAVEFORM_CONTAINER_LAYERS.lock().unwrap();
  let mut decor_guard = DECOR_LAYER.lock().unwrap();
  if decor_guard.is_none() {
    let decor: id = msg_send![class!(CALayer), layer];
    let _: () = msg_send![decor, setFrame: bounds];
    let _: () = msg_send![decor, setOpaque: false];
    let clear: id = msg_send![class!(NSColor), clearColor];
    let clear_cg: *mut Object = msg_send![clear, CGColor];
    let _: () = msg_send![decor, setBackgroundColor: clear_cg];
    let _: () = msg_send![layer, addSublayer: decor];
    *decor_guard = Some(ViewPtr(decor));
  } else if let Some(decor) = decor_guard.as_ref() {
    let _: () = msg_send![decor.0, setFrame: bounds];
  }

  if guard[0].is_none() || guard[1].is_none() {
    let left = create_waveform_container_layer(layer, left_frame);
    let right = create_waveform_container_layer(layer, right_frame);
    guard[0] = Some(ViewPtr(left));
    guard[1] = Some(ViewPtr(right));
  } else {
    if let Some(left) = guard[0].as_ref() {
      let _: () = msg_send![left.0, setFrame: left_frame];
    }
    if let Some(right) = guard[1].as_ref() {
      let _: () = msg_send![right.0, setFrame: right_frame];
    }
  }
}

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
  let metal_layer_class = Class::get("CAMetalLayer").unwrap_or_else(|| class!(CALayer));
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

fn choose_surface_config(
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
      module: &shader,
      entry_point: "cs_peaks",
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

    let initial_size = *size_for_thread.lock().unwrap();
    let mut config = choose_surface_config(&surface, &adapter, initial_size.0, initial_size.1);
    surface.configure(&device, &config);

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
      label: Some("sujay-native-ui-waveform-render"),
      layout: Some(&render_pipeline_layout),
      vertex: wgpu::VertexState {
        module: &shader,
        entry_point: "vs_fullscreen",
        buffers: &[],
        compilation_options: wgpu::PipelineCompilationOptions::default(),
      },
      fragment: Some(wgpu::FragmentState {
        module: &shader,
        entry_point: "fs_waveform",
        targets: &[Some(wgpu::ColorTargetState {
          format: config.format,
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

    let create_deck_state = |deck_index: usize| -> DeckGpuState {
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
        layout: &compute_bind_group_layout,
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
    };

    let mut deck_states = [create_deck_state(0), create_deck_state(1)];

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

    let mut last_versions = [0_u64, 0_u64];
    let mut frame_counter: f32 = 0.0;

    while running_for_thread.load(Ordering::Relaxed) {
      for deck_index in 0..2 {
        let version = WAVEFORM_VERSIONS[deck_index].load(Ordering::Relaxed);
        if version == last_versions[deck_index] {
          continue;
        }

        let samples = {
          let guard = WAVEFORMS.lock().unwrap();
          guard[deck_index].clone()
        };

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
          layout: &compute_bind_group_layout,
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

      for deck in deck_states.iter() {
        let params = encode_u32x4(deck.sample_count, PEAK_BINS, 0, 0);
        queue.write_buffer(&deck.compute_params_buffer, 0, &params);
      }

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
  drop(guard);
  unsafe { redraw_deck_layer(deck_index); }
}

pub fn set_deck_progress(deck_index: usize, progress: f32, duration: f32) {
  if deck_index > 1 {
    return;
  }
  let mut guard = DECK_VISUALS.lock().unwrap();
  guard[deck_index].progress = progress.clamp(0.0, 1.0);
  guard[deck_index].duration = if duration.is_finite() && duration > 0.0 { duration } else { 0.0 };
  drop(guard);
  unsafe { redraw_deck_layer(deck_index); }
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
  drop(guard);

  unsafe { redraw_deck_layer(deck_index); }
}

pub fn set_console_state(state: ConsoleVisualState) {
  let mut guard = CONSOLE_VISUAL.lock().unwrap();
  *guard = state;
  drop(guard);
  unsafe { redraw_all_decks(); }
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

  {
    let mut layers = WAVEFORM_CONTAINER_LAYERS.lock().unwrap();
    layers[0] = None;
    layers[1] = None;
  }
  *DECOR_LAYER.lock().unwrap() = None;

  let frame = html_frame_to_nsview_frame(parent_view, x, y, width, height);
  let view: id = msg_send![class!(NSView), alloc];
  let view: id = msg_send![view, initWithFrame: frame];

  // Make it layer-backed and bind CAMetalLayer.
  let _: () = msg_send![view, setWantsLayer: true];
  let metal_layer = create_metal_layer(parent_view, frame);
  let _: () = msg_send![view, setLayer: metal_layer];
  layout_waveform_containers(view);

  // Add as subview of the Electron content view
  // Keep native view above WebContents so it is always visible during migration.
  let _: () = msg_send![parent_view, addSubview: view positioned: 1_i64 relativeTo: nil];

  // NOTE: wgpu Metal surface on macOS must be touched on UI thread.
  // Keep renderer thread disabled for now and use CALayer-based waveform drawing.
  let (_px_w, _px_h) = logical_to_physical_size(parent_view, width, height);

  *CHILD_VIEW.lock().unwrap() = Some(ViewPtr(view));
  redraw_all_decks();
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
    layout_waveform_containers(view_ptr.0 as id);
    redraw_all_decks();
  }
}

/// Remove the attached view.
pub unsafe fn detach() {
  stop_renderer();

  {
    let mut layers = WAVEFORM_CONTAINER_LAYERS.lock().unwrap();
    for layer in layers.iter_mut() {
      if let Some(ptr) = layer.take() {
        let _: () = msg_send![ptr.0, removeFromSuperlayer];
      }
    }
  }

  {
    let mut decor = DECOR_LAYER.lock().unwrap();
    if let Some(ptr) = decor.take() {
      let _: () = msg_send![ptr.0, removeFromSuperlayer];
    }
  }

  let mut guard = CHILD_VIEW.lock().unwrap();
  if let Some(view_ptr) = guard.take() {
    let _: () = msg_send![view_ptr.0, removeFromSuperview];
  }
}
