//! C ABI over [`sujay_core::Core`] for the Swift host.
//!
//! Single-threaded by contract: every function is called from the host's
//! main thread. State crosses at three rates — commands as plain functions,
//! the fast numeric state as one POD struct per frame, and the slow textual
//! state (titles, cues, library, preferences) as JSON only when the returned
//! [`SujayTick`] says it changed. Bulk buffers (waveform, colours, beat grid)
//! are copied out into caller-owned memory.
//!
//! The header is hand-written: `Vendor/SujayCore/include/sujay.h`. Keep the
//! two in step.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use serde::Serialize;
use sujay_core::state::{DeckConsoleVisualState, DeckCueVisualState, PreferencesState};
use sujay_core::Core;

/// Opaque handle.
pub struct SujayCore {
  core: Core,
  /// Last slow-console state handed to the host, to report changes.
  last_slow: SlowConsole,
  slow_initialised: bool,
}

#[derive(Clone, Default, PartialEq, Serialize)]
struct SlowDeck {
  title: String,
  bpm_text: String,
  cues: Vec<DeckCueVisualState>,
}

impl SlowDeck {
  fn from_deck(deck: &DeckConsoleVisualState) -> Self {
    Self {
      title: deck.title.clone(),
      bpm_text: deck.bpm_text.clone(),
      cues: deck.rekordbox_cues.clone(),
    }
  }
}

#[derive(Clone, Default, PartialEq, Serialize)]
struct SlowConsole {
  deck_a: SlowDeck,
  deck_b: SlowDeck,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SujayTick {
  /// Titles, BPM text or cues changed: re-read `sujay_core_console_json`.
  pub console: u8,
  /// Library list changed: re-read `sujay_core_library_json`.
  pub library: u8,
  /// Preferences changed: re-read `sujay_core_preferences_json`.
  pub preferences: u8,
  /// A decoded track is waiting for the engine; tick again within ~1 ms.
  pub retry_soon: u8,
  /// Waveform, colours or beat grid replaced for deck A / B.
  pub deck: [u8; 2],
  pub _pad: [u8; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SujayDeckSnapshot {
  pub position_frames: f32,
  pub total_frames: f32,
  pub sample_rate: f32,
  pub peak: f32,
  pub gain: f32,
  pub bpm: f32,
  pub loop_start: f32,
  pub loop_end: f32,
  pub loop_beats: f32,
  pub playing: u8,
  pub cue_enabled: u8,
  pub eq_low: u8,
  pub eq_mid: u8,
  pub eq_high: u8,
  pub loop_enabled: u8,
  /// A track is loaded (progress is meaningful).
  pub loaded: u8,
  pub _pad: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SujaySnapshot {
  pub deck: [SujayDeckSnapshot; 2],
  pub master_tempo: f32,
  pub crossfader: f32,
  pub cpu_percent: f32,
  pub mic_peak: f32,
  pub mem_mb: u64,
  pub rec_elapsed_secs: u32,
  pub mic_available: u8,
  pub mic_enabled: u8,
  pub is_recording: u8,
  pub _pad: u8,
}

fn b(v: bool) -> u8 {
  v as u8
}

fn deck_id(deck: u8) -> u8 {
  if deck <= 1 {
    1
  } else {
    2
  }
}

fn to_c_string(s: String) -> *mut c_char {
  CString::new(s)
    .map(CString::into_raw)
    .unwrap_or(std::ptr::null_mut())
}

unsafe fn c_str<'a>(ptr: *const c_char) -> Option<&'a str> {
  if ptr.is_null() {
    return None;
  }
  CStr::from_ptr(ptr).to_str().ok()
}

// ── Lifecycle ────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn sujay_core_new() -> *mut SujayCore {
  Box::into_raw(Box::new(SujayCore {
    core: Core::new(),
    last_slow: SlowConsole::default(),
    slow_initialised: false,
  }))
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_free(handle: *mut SujayCore) {
  if !handle.is_null() {
    drop(Box::from_raw(handle));
  }
}

/// Start the audio engine and the library load. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_start(handle: *mut SujayCore) -> i32 {
  match (*handle).core.start() {
    Ok(()) => 0,
    Err(err) => {
      eprintln!("[ffi] start failed: {err}");
      1
    }
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_shutdown(handle: *mut SujayCore) {
  (*handle).core.shutdown();
}

// ── Per-frame ────────────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn sujay_core_tick(handle: *mut SujayCore) -> SujayTick {
  let this = &mut *handle;
  let tick = this.core.tick();

  let mut console_changed = false;
  if tick.console || !this.slow_initialised {
    let console = this.core.console_state();
    let slow = SlowConsole {
      deck_a: SlowDeck::from_deck(&console.deck_a),
      deck_b: SlowDeck::from_deck(&console.deck_b),
    };
    if !this.slow_initialised || slow != this.last_slow {
      this.last_slow = slow;
      this.slow_initialised = true;
      console_changed = true;
    }
  }

  SujayTick {
    console: b(console_changed),
    library: b(tick.library),
    preferences: b(tick.preferences),
    retry_soon: b(tick.retry_soon),
    deck: [
      b(tick.waveform[0] || tick.markers[0]),
      b(tick.waveform[1] || tick.markers[1]),
    ],
    _pad: [0; 2],
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_snapshot(handle: *const SujayCore, out: *mut SujaySnapshot) {
  let this = &*handle;
  let console = this.core.console_state();
  let mut snapshot = SujaySnapshot {
    master_tempo: console.master_tempo,
    crossfader: console.crossfader,
    cpu_percent: console.titlebar.cpu_percent,
    mic_peak: console.titlebar.mic_peak,
    mem_mb: console.titlebar.mem_mb,
    rec_elapsed_secs: console.titlebar.rec_elapsed_secs,
    mic_available: b(console.titlebar.mic_available),
    mic_enabled: b(console.titlebar.mic_enabled),
    is_recording: b(console.titlebar.is_recording),
    ..SujaySnapshot::default()
  };
  for (idx, deck) in [&console.deck_a, &console.deck_b].into_iter().enumerate() {
    let buffers = this.core.deck(idx as u8 + 1);
    let (position, total, sample_rate) = buffers.progress.unwrap_or((0.0, 0.0, 0.0));
    snapshot.deck[idx] = SujayDeckSnapshot {
      position_frames: position,
      total_frames: total,
      sample_rate,
      peak: deck.peak,
      gain: deck.gain,
      bpm: deck.bpm,
      loop_start: deck.loop_start,
      loop_end: deck.loop_end,
      loop_beats: deck.loop_beats,
      playing: b(deck.playing),
      cue_enabled: b(deck.cue_enabled),
      eq_low: b(deck.eq_low),
      eq_mid: b(deck.eq_mid),
      eq_high: b(deck.eq_high),
      loop_enabled: b(deck.loop_enabled),
      loaded: b(buffers.progress.is_some()),
      _pad: 0,
    };
  }
  *out = snapshot;
}

// ── Slow state (JSON; free with sujay_string_free) ──────────────────────────

#[no_mangle]
pub unsafe extern "C" fn sujay_core_console_json(handle: *const SujayCore) -> *mut c_char {
  let this = &*handle;
  let console = this.core.console_state();
  let slow = SlowConsole {
    deck_a: SlowDeck::from_deck(&console.deck_a),
    deck_b: SlowDeck::from_deck(&console.deck_b),
  };
  to_c_string(serde_json::to_string(&slow).unwrap_or_default())
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_library_json(handle: *const SujayCore) -> *mut c_char {
  to_c_string(serde_json::to_string((*handle).core.library_state()).unwrap_or_default())
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_preferences_json(handle: *const SujayCore) -> *mut c_char {
  to_c_string(serde_json::to_string(&(*handle).core.preferences_state()).unwrap_or_default())
}

#[no_mangle]
pub unsafe extern "C" fn sujay_string_free(s: *mut c_char) {
  if !s.is_null() {
    drop(CString::from_raw(s));
  }
}

// ── Bulk buffers ─────────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn sujay_core_waveform_len(handle: *const SujayCore, deck: u8) -> usize {
  (*handle).core.deck(deck_id(deck)).waveform.len()
}

/// Copy up to `cap` waveform peaks (0..1) into `out`; returns the count copied.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_copy_waveform(
  handle: *const SujayCore,
  deck: u8,
  out: *mut f32,
  cap: usize,
) -> usize {
  let src = &(*handle).core.deck(deck_id(deck)).waveform;
  let n = src.len().min(cap);
  if n > 0 {
    std::ptr::copy_nonoverlapping(src.as_ptr(), out, n);
  }
  n
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_waveform_colors_len(
  handle: *const SujayCore,
  deck: u8,
) -> usize {
  (*handle).core.deck(deck_id(deck)).waveform_colors.len()
}

/// Copy up to `cap` RGB triplets (3 bytes each) into `out`; returns the count
/// of triplets copied.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_copy_waveform_colors(
  handle: *const SujayCore,
  deck: u8,
  out: *mut u8,
  cap: usize,
) -> usize {
  let src = &(*handle).core.deck(deck_id(deck)).waveform_colors;
  let n = src.len().min(cap);
  if n > 0 {
    std::ptr::copy_nonoverlapping(src.as_ptr() as *const u8, out, n * 3);
  }
  n
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_beats_len(handle: *const SujayCore, deck: u8) -> usize {
  (*handle).core.deck(deck_id(deck)).beats.len()
}

/// Copy up to `cap` beat positions (audio frames) into `out`; returns the count copied.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_copy_beats(
  handle: *const SujayCore,
  deck: u8,
  out: *mut f32,
  cap: usize,
) -> usize {
  let src = &(*handle).core.deck(deck_id(deck)).beats;
  let n = src.len().min(cap);
  if n > 0 {
    std::ptr::copy_nonoverlapping(src.as_ptr(), out, n);
  }
  n
}

/// Intro / outro markers in audio frames. Bit 0 of the result = intro valid,
/// bit 1 = outro valid.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_deck_markers(
  handle: *const SujayCore,
  deck: u8,
  intro: *mut f32,
  outro: *mut f32,
) -> u8 {
  let buffers = (*handle).core.deck(deck_id(deck));
  let mut flags = 0;
  if let Some(v) = buffers.intro {
    *intro = v;
    flags |= 1;
  }
  if let Some(v) = buffers.outro {
    *outro = v;
    flags |= 2;
  }
  flags
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn sujay_core_play(handle: *const SujayCore, deck: u8) {
  (*handle).core.play(deck_id(deck));
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_stop(handle: *const SujayCore, deck: u8) {
  (*handle).core.stop(deck_id(deck));
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_set_crossfader(handle: *const SujayCore, position: f32) {
  (*handle).core.set_crossfader(position);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_set_master_tempo(handle: *const SujayCore, bpm: f32) {
  (*handle).core.set_master_tempo(bpm);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_set_deck_gain(handle: *const SujayCore, deck: u8, gain: f32) {
  (*handle).core.set_deck_gain(deck_id(deck), gain);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_set_cue(handle: *const SujayCore, deck: u8, enabled: bool) {
  (*handle).core.set_cue(deck_id(deck), enabled);
}

/// `band`: 0 = low, 1 = mid, 2 = high.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_set_eq(
  handle: *const SujayCore,
  deck: u8,
  band: u8,
  kill: bool,
) {
  let band = match band {
    0 => "low",
    1 => "mid",
    _ => "high",
  };
  (*handle).core.set_eq(deck_id(deck), band, kill);
}

/// `position` is a fraction of the track (0..1).
#[no_mangle]
pub unsafe extern "C" fn sujay_core_seek(handle: *const SujayCore, deck: u8, position: f32) {
  (*handle).core.seek(deck_id(deck), position);
}

/// Positions are fractions of the track (0..1); `loop_end < 0` means no loop.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_recall_cue(
  handle: *const SujayCore,
  deck: u8,
  position: f32,
  loop_end: f32,
) {
  let loop_end = (loop_end >= 0.0).then_some(loop_end);
  (*handle).core.recall_cue(deck_id(deck), position, loop_end);
}

/// `beats <= 0` clears the loop.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_toggle_loop(handle: *const SujayCore, deck: u8, beats: f32) {
  (*handle).core.toggle_loop(deck_id(deck), beats);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_set_mic_enabled(handle: *const SujayCore, enabled: bool) {
  (*handle).core.set_mic_enabled(enabled);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_start_recording(handle: *const SujayCore) {
  (*handle).core.start_recording();
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_stop_recording(handle: *const SujayCore) {
  (*handle).core.stop_recording();
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_load_file(
  handle: *const SujayCore,
  deck: u8,
  path: *const c_char,
) {
  if let Some(path) = c_str(path) {
    (*handle).core.load_file(deck_id(deck), PathBuf::from(path));
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_core_refresh_audio_devices(handle: *mut SujayCore) {
  (*handle).core.refresh_audio_devices();
}

/// Apply a `PreferencesState` JSON document. Returns 0 on success, 1 on parse error.
#[no_mangle]
pub unsafe extern "C" fn sujay_core_apply_preferences_json(
  handle: *mut SujayCore,
  json: *const c_char,
) -> i32 {
  let Some(json) = c_str(json) else {
    return 1;
  };
  match serde_json::from_str::<PreferencesState>(json) {
    Ok(state) => {
      (*handle).core.apply_preferences(state);
      0
    }
    Err(err) => {
      eprintln!("[ffi] bad preferences json: {err}");
      1
    }
  }
}
