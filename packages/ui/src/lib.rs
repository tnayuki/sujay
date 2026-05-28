use napi_derive::napi;

mod ui_state;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod renderer_wgpu_shared;

#[cfg(target_os = "macos")]
#[path = "renderer.rs"]
mod renderer;
#[cfg(target_os = "windows")]
#[path = "renderer_windows.rs"]
mod renderer;
#[cfg(all(target_os = "macos", feature = "gpui-preview"))]
mod gpui_preview;

#[napi(object)]
pub struct NativeDeckConsoleState {
  pub title: String,
  pub time_text: String,
  pub bpm_text: String,
  pub bpm: f64,
  pub playing: bool,
  pub loop_enabled: bool,
  pub loop_beats: f64,
  pub loop_start: f64,
  pub loop_end: f64,
  pub cue_enabled: bool,
  pub eq_low: bool,
  pub eq_mid: bool,
  pub eq_high: bool,
  pub gain: f64,
  pub peak: f64,
}

#[napi(object)]
pub struct NativeConsoleState {
  pub deck_a: NativeDeckConsoleState,
  pub deck_b: NativeDeckConsoleState,
  pub master_tempo: f64,
  pub crossfader: f64,
}

#[napi]
pub fn addon_version() -> String {
  env!("CARGO_PKG_VERSION").to_string()
}

#[napi]
pub fn is_gpui_enabled() -> bool {
  cfg!(all(target_os = "macos", feature = "gpui-preview"))
}

#[napi]
pub fn launch_gpui_preview() -> bool {
  #[cfg(all(target_os = "macos", feature = "gpui-preview"))]
  {
    return gpui_preview::launch_preview();
  }

  #[allow(unreachable_code)]
  false
}

/// Attach a native GPU-rendered view to an Electron BrowserWindow.
///
/// `native_handle` is the Buffer from `BrowserWindow.getNativeWindowHandle()`.
/// On macOS this is a pointer to the NSView (content view).
/// `x`, `y`, `width`, `height` define the frame within the parent window.
#[napi]
pub fn attach(native_handle: napi::bindgen_prelude::Buffer, x: f64, y: f64, width: f64, height: f64) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    let handle_bytes = native_handle.as_ref();
    // Electron's getNativeWindowHandle() returns a pointer as bytes
    let ptr = if handle_bytes.len() == 8 {
      u64::from_ne_bytes(handle_bytes.try_into().unwrap()) as *mut std::ffi::c_void
    } else if handle_bytes.len() == 4 {
      u32::from_ne_bytes(handle_bytes.try_into().unwrap()) as *mut std::ffi::c_void
    } else {
      return;
    };

    unsafe {
      renderer::attach(ptr, x, y, width, height);
    }
  }
}

/// Resize / reposition the native view within the parent window.
#[napi]
pub fn set_frame(x: f64, y: f64, width: f64, height: f64) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  unsafe {
    renderer::set_frame(x, y, width, height);
  }
}

/// Detach and destroy the native view.
#[napi]
pub fn detach() {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  unsafe {
    renderer::detach();
  }
}

/// Set decimated waveform samples for the specified deck.
#[napi]
pub fn set_waveform(deck: u32, samples: Vec<f64>) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    let deck_index = if deck <= 1 { 0 } else { 1 };
    let samples_f32 = samples.into_iter().map(|v| v as f32).collect();
    renderer::set_waveform(deck_index, samples_f32);
  }
}

#[napi]
pub fn set_deck_progress(deck: u32, position_frames: f64, total_frames: f64, audio_sample_rate: f64) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    let deck_index = if deck <= 1 { 0 } else { 1 };
    renderer::set_deck_progress(deck_index, position_frames as f32, total_frames as f32, audio_sample_rate as f32);
  }
}

#[napi]
pub fn set_deck_markers(deck: u32, beats: Vec<f64>, intro: Option<f64>, outro: Option<f64>) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    let deck_index = if deck <= 1 { 0 } else { 1 };
    let beats_f32 = beats.into_iter().map(|v| v as f32).collect();
    renderer::set_deck_markers(
      deck_index,
      beats_f32,
      intro.map(|v| v as f32),
      outro.map(|v| v as f32),
    );
  }
}

#[napi]
pub fn set_console_state(state: NativeConsoleState) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    renderer::set_console_state(ui_state::ConsoleVisualState {
      deck_a: ui_state::DeckConsoleVisualState {
        title: state.deck_a.title,
        time_text: state.deck_a.time_text,
        bpm_text: state.deck_a.bpm_text,
        bpm: state.deck_a.bpm as f32,
        playing: state.deck_a.playing,
        loop_enabled: state.deck_a.loop_enabled,
        loop_beats: state.deck_a.loop_beats as f32,
        loop_start: state.deck_a.loop_start as f32,
        loop_end: state.deck_a.loop_end as f32,
        cue_enabled: state.deck_a.cue_enabled,
        eq_low: state.deck_a.eq_low,
        eq_mid: state.deck_a.eq_mid,
        eq_high: state.deck_a.eq_high,
        gain: state.deck_a.gain as f32,
        peak: state.deck_a.peak as f32,
      },
      deck_b: ui_state::DeckConsoleVisualState {
        title: state.deck_b.title,
        time_text: state.deck_b.time_text,
        bpm_text: state.deck_b.bpm_text,
        bpm: state.deck_b.bpm as f32,
        playing: state.deck_b.playing,
        loop_enabled: state.deck_b.loop_enabled,
        loop_beats: state.deck_b.loop_beats as f32,
        loop_start: state.deck_b.loop_start as f32,
        loop_end: state.deck_b.loop_end as f32,
        cue_enabled: state.deck_b.cue_enabled,
        eq_low: state.deck_b.eq_low,
        eq_mid: state.deck_b.eq_mid,
        eq_high: state.deck_b.eq_high,
        gain: state.deck_b.gain as f32,
        peak: state.deck_b.peak as f32,
      },
      master_tempo: state.master_tempo as f32,
      crossfader: state.crossfader as f32,
    });
  }
}

/// Set deck artwork from RGBA pixel data.
#[napi]
pub fn set_deck_artwork(deck: u32, width: u32, height: u32, rgba: napi::bindgen_prelude::Buffer) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    let deck_index = if deck <= 1 { 0 } else { 1 };
    renderer::set_deck_artwork(deck_index, width, height, rgba.to_vec());
  }
}

/// Clear deck artwork (show placeholder).
#[napi]
pub fn clear_deck_artwork(deck: u32) {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    let deck_index = if deck <= 1 { 0 } else { 1 };
    renderer::clear_deck_artwork(deck_index);
  }
}

#[napi(object)]
pub struct NativeUiAction {
  /// Action type: "play", "stop", "crossfader", "master_tempo", "cue", "eq", "loop", "seek"
  pub action: String,
  /// Deck number (1 or 2), 0 if N/A
  pub deck: u32,
  /// Float value (crossfader pos, tempo, seek pos, loop beats)
  pub value: f64,
  /// String param (eq band: "high"/"mid"/"low")
  pub param: String,
}

/// Poll pending UI actions from egui interactions.
#[napi]
pub fn poll_actions() -> Vec<NativeUiAction> {
  #[cfg(any(target_os = "macos", target_os = "windows"))]
  {
    renderer::drain_actions()
      .into_iter()
      .map(|a| match a {
        renderer::UiAction::Play(d) => NativeUiAction {
          action: "play".into(), deck: d as u32, value: 0.0, param: String::new(),
        },
        renderer::UiAction::Stop(d) => NativeUiAction {
          action: "stop".into(), deck: d as u32, value: 0.0, param: String::new(),
        },
        renderer::UiAction::SetCrossfader(v) => NativeUiAction {
          action: "crossfader".into(), deck: 0, value: v as f64, param: String::new(),
        },
        renderer::UiAction::SetMasterTempo(v) => NativeUiAction {
          action: "master_tempo".into(), deck: 0, value: v as f64, param: String::new(),
        },
        renderer::UiAction::SetCue(d, enabled) => NativeUiAction {
          action: "cue".into(), deck: d as u32, value: if enabled { 1.0 } else { 0.0 }, param: String::new(),
        },
        renderer::UiAction::SetEq(d, band, enabled) => NativeUiAction {
          action: "eq".into(), deck: d as u32, value: if enabled { 1.0 } else { 0.0 }, param: band.to_string(),
        },
        renderer::UiAction::ToggleLoop(d, beats) => NativeUiAction {
          action: "loop".into(), deck: d as u32, value: beats as f64, param: String::new(),
        },
        renderer::UiAction::Seek(d, pos) => NativeUiAction {
          action: "seek".into(), deck: d as u32, value: pos as f64, param: String::new(),
        },
        renderer::UiAction::SetDeckGain(d, gain) => NativeUiAction {
          action: "deck_gain".into(), deck: d as u32, value: gain as f64, param: String::new(),
        },
      })
      .collect()
  }
  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  {
    vec![]
  }
}
