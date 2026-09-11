//! C ABI over the two things still in Rust: the audio engine
//! (`sujay_audio::engine_core::AudioEngineCore`) and the rekordbox reader
//! (`sujay_library`). Everything else — preferences, decoding, the library
//! model, beat-loop maths, state for the UI — lives in Swift.
//!
//! Single-threaded by contract for the engine handle. Strings returned as
//! `char *` are freed with `sujay_string_free`. The header is hand-written:
//! `Vendor/SujayCore/include/sujay.h`; keep the two in step.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, CStr, CString};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use sujay_audio::engine_core::{
  list_output_devices, AudioEngineCore, DeviceConfigCore, EngineStateUpdate,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn b(v: bool) -> u8 {
  v as u8
}

fn deck_id(deck: u8) -> u32 {
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

fn json<T: Serialize>(value: &T) -> *mut c_char {
  to_c_string(serde_json::to_string(value).unwrap_or_default())
}

#[no_mangle]
pub unsafe extern "C" fn sujay_string_free(s: *mut c_char) {
  if !s.is_null() {
    drop(CString::from_raw(s));
  }
}

// ── Engine ───────────────────────────────────────────────────────────────────

pub struct SujayEngine {
  engine: AudioEngineCore,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SujayDeckState {
  /// Audio frames; meaningful when `loaded`.
  pub position_frames: f64,
  pub total_frames: f64,
  pub peak: f32,
  pub peak_hold: f32,
  pub gain: f32,
  /// Track BPM, 0 when unknown.
  pub bpm: f32,
  /// Loop bounds in audio frames.
  pub loop_start: f32,
  pub loop_end: f32,
  pub playing: u8,
  pub cue_enabled: u8,
  pub eq_low: u8,
  pub eq_mid: u8,
  pub eq_high: u8,
  pub loop_enabled: u8,
  pub loaded: u8,
  pub _pad: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SujayEngineState {
  pub deck: [SujayDeckState; 2],
  pub master_tempo: f32,
  pub crossfader: f32,
  pub mic_peak: f32,
  pub sample_rate: f32,
  pub is_crossfading: u8,
  pub mic_available: u8,
  pub mic_enabled: u8,
  pub is_recording: u8,
}

#[no_mangle]
pub extern "C" fn sujay_engine_new(sample_rate: u32) -> *mut SujayEngine {
  // The engine's periodic callback is not used: the host polls `sujay_engine_state`.
  match AudioEngineCore::new(Some(sample_rate), Arc::new(|_: EngineStateUpdate| {})) {
    Ok(engine) => Box::into_raw(Box::new(SujayEngine { engine })),
    Err(err) => {
      eprintln!("[ffi] engine start failed: {err}");
      std::ptr::null_mut()
    }
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_free(handle: *mut SujayEngine) {
  if !handle.is_null() {
    let engine = Box::from_raw(handle);
    engine.engine.close();
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_sample_rate(handle: *const SujayEngine) -> u32 {
  (*handle).engine.sample_rate
}

/// `device_id` may be NULL for the system default. `main` / `cue` are two
/// channel indices each; -1 disables a side. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_configure_device(
  handle: *const SujayEngine,
  device_id: *const c_char,
  main: *const i32,
  cue: *const i32,
) -> i32 {
  let config = DeviceConfigCore {
    device_id: c_str(device_id).map(str::to_owned),
    main_channels: Some(vec![*main, *main.add(1)]),
    cue_channels: Some(vec![*cue, *cue.add(1)]),
  };
  match (*handle).engine.configure_device(config) {
    Ok(()) => 0,
    Err(err) => {
      eprintln!("[ffi] configure_device failed: {err}");
      1
    }
  }
}

/// Load interleaved stereo PCM at the engine's sample rate. `bpm <= 0` means
/// unknown; `beats` are audio frame indices. The engine hands the buffer to
/// its processing thread without blocking it, so this retries for up to
/// 250 ms while that thread holds the state; returns 0 on success, 1 if it
/// stayed busy.
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_load_track(
  handle: *const SujayEngine,
  deck: u8,
  pcm: *const f32,
  frames: usize,
  bpm: f32,
  beats: *const f32,
  beat_count: usize,
  track_id: *const c_char,
) -> i32 {
  let engine = &(*handle).engine;
  let mut pcm = Some(std::slice::from_raw_parts(pcm, frames * 2).to_vec());
  let beats = if beat_count > 0 {
    std::slice::from_raw_parts(beats, beat_count).to_vec()
  } else {
    Vec::new()
  };
  let bpm = (bpm > 0.0).then_some(bpm);
  let mut track_id = c_str(track_id).map(str::to_owned);
  let deadline = Instant::now() + Duration::from_millis(250);
  loop {
    if engine.try_load_track(deck_id(deck), &mut pcm, bpm, beats.clone(), &mut track_id) {
      return 0;
    }
    if Instant::now() > deadline {
      return 1;
    }
    std::thread::sleep(Duration::from_millis(1));
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_play(handle: *const SujayEngine, deck: u8) {
  let _ = (*handle).engine.play(deck_id(deck));
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_stop(handle: *const SujayEngine, deck: u8) {
  let _ = (*handle).engine.stop(deck_id(deck));
}

/// `position` is a fraction of the track (0..1).
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_seek(handle: *const SujayEngine, deck: u8, position: f64) {
  let _ = (*handle).engine.seek(deck_id(deck), position);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_crossfader(handle: *const SujayEngine, position: f64) {
  let _ = (*handle).engine.set_crossfader_position(position);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_master_tempo(handle: *const SujayEngine, bpm: f64) {
  let _ = (*handle).engine.set_master_tempo(bpm);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_deck_gain(
  handle: *const SujayEngine,
  deck: u8,
  gain: f64,
) {
  let _ = (*handle).engine.set_deck_gain(deck_id(deck), gain);
}

/// `band`: 0 = low, 1 = mid, 2 = high.
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_eq(
  handle: *const SujayEngine,
  deck: u8,
  band: u8,
  kill: bool,
) {
  let band = match band {
    0 => "low",
    1 => "mid",
    _ => "high",
  };
  let _ = (*handle).engine.set_eq_cut(deck_id(deck), band, kill);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_cue(handle: *const SujayEngine, deck: u8, enabled: bool) {
  let _ = (*handle)
    .engine
    .set_deck_cue_enabled(deck_id(deck), enabled);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_mic_enabled(handle: *const SujayEngine, enabled: bool) {
  let _ = (*handle).engine.set_mic_enabled(enabled);
}

/// Loop bounds as fractions of the track (0..1).
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_loop(
  handle: *const SujayEngine,
  deck: u8,
  start: f64,
  end: f64,
  enabled: bool,
) {
  let _ = (*handle)
    .engine
    .set_loop(deck_id(deck), start, end, enabled);
}

/// Loop bounds in seconds; the playhead is moved inside the loop.
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_set_beat_loop(
  handle: *const SujayEngine,
  deck: u8,
  start_seconds: f64,
  end_seconds: f64,
) {
  let _ = (*handle)
    .engine
    .set_beat_loop(deck_id(deck), start_seconds, end_seconds);
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_clear_loop(handle: *const SujayEngine, deck: u8) {
  let _ = (*handle).engine.clear_loop(deck_id(deck));
}

/// `format`: 0 = wav, 1 = ogg. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn sujay_engine_start_recording(
  handle: *const SujayEngine,
  path: *const c_char,
  format: u8,
) -> i32 {
  let Some(path) = c_str(path) else {
    return 1;
  };
  let format = if format == 1 { "ogg" } else { "wav" };
  match (*handle).engine.start_recording(path.to_owned(), format) {
    Ok(()) => 0,
    Err(err) => {
      eprintln!("[ffi] start_recording failed: {err}");
      1
    }
  }
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_stop_recording(handle: *const SujayEngine) {
  let _ = (*handle).engine.stop_recording();
}

#[no_mangle]
pub unsafe extern "C" fn sujay_engine_state(
  handle: *const SujayEngine,
  out: *mut SujayEngineState,
) {
  let s = (*handle).engine.get_state();
  let deck = |position: Option<f64>,
              total: Option<f64>,
              peak: f64,
              peak_hold: f64,
              gain: f64,
              bpm: Option<f64>,
              loop_state: sujay_audio::engine_core::LoopState,
              playing: bool,
              cue: bool,
              eq: (bool, bool, bool)|
   -> SujayDeckState {
    SujayDeckState {
      position_frames: position.unwrap_or(0.0),
      total_frames: total.unwrap_or(0.0),
      peak: peak as f32,
      peak_hold: peak_hold as f32,
      gain: gain as f32,
      bpm: bpm.unwrap_or(0.0) as f32,
      loop_start: loop_state.start as f32,
      loop_end: loop_state.end as f32,
      playing: b(playing),
      cue_enabled: b(cue),
      eq_low: b(eq.0),
      eq_mid: b(eq.1),
      eq_high: b(eq.2),
      loop_enabled: b(loop_state.enabled),
      loaded: b(total.is_some()),
      _pad: 0,
    }
  };
  *out = SujayEngineState {
    deck: [
      deck(
        s.deck_a_position,
        s.deck_a_total_frames,
        s.deck_a_peak,
        s.deck_a_peak_hold,
        s.deck_a_gain,
        s.deck_a_bpm,
        s.deck_a_loop,
        s.deck_a_playing,
        s.deck_a_cue_enabled,
        (
          s.deck_a_eq_cut.low,
          s.deck_a_eq_cut.mid,
          s.deck_a_eq_cut.high,
        ),
      ),
      deck(
        s.deck_b_position,
        s.deck_b_total_frames,
        s.deck_b_peak,
        s.deck_b_peak_hold,
        s.deck_b_gain,
        s.deck_b_bpm,
        s.deck_b_loop,
        s.deck_b_playing,
        s.deck_b_cue_enabled,
        (
          s.deck_b_eq_cut.low,
          s.deck_b_eq_cut.mid,
          s.deck_b_eq_cut.high,
        ),
      ),
    ],
    master_tempo: s.master_tempo as f32,
    crossfader: s.crossfader_position as f32,
    mic_peak: s.mic_peak as f32,
    sample_rate: s.sample_rate as f32,
    is_crossfading: b(s.is_crossfading),
    mic_available: b(s.mic_available),
    mic_enabled: b(s.mic_enabled),
    is_recording: b(s.is_recording),
  };
}

#[derive(Serialize)]
struct DeviceJson {
  name: String,
  max_output_channels: u16,
}

/// `[{name, max_output_channels}]`, sorted by name. Reads the HAL property
/// API; creates no AudioUnit.
#[no_mangle]
pub extern "C" fn sujay_list_output_devices_json() -> *mut c_char {
  let devices: Vec<DeviceJson> = list_output_devices()
    .unwrap_or_default()
    .into_iter()
    .map(|(name, max_output_channels)| DeviceJson {
      name,
      max_output_channels,
    })
    .collect();
  json(&devices)
}

// ── Rekordbox ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TrackJson {
  id: String,
  title: String,
  artist: String,
  album: String,
  bpm: Option<f32>,
  duration_seconds: Option<f32>,
  rating: Option<i32>,
  tags: Option<String>,
  release_date: Option<String>,
  file_path: String,
}

#[derive(Serialize)]
struct PlaylistJson {
  id: String,
  name: String,
  parent_id: String,
  is_folder: bool,
  track_ids: Vec<String>,
}

#[derive(Serialize)]
struct LibraryJson {
  master_db_path: String,
  tracks: Vec<TrackJson>,
  playlists: Vec<PlaylistJson>,
}

#[derive(Serialize)]
struct ErrorJson {
  error: String,
}

/// The browse list without per-track analysis. `master_db` may be NULL to
/// find the newest rekordbox `master.db` under ~/Library/Pioneer. On failure
/// the JSON is `{"error": "..."}`.
#[no_mangle]
pub unsafe extern "C" fn sujay_library_load_json(master_db: *const c_char) -> *mut c_char {
  let loaded = match c_str(master_db) {
    Some(path) => sujay_library::RekordboxLibrary::load_from_master_db_fast(path),
    None => sujay_library::RekordboxLibrary::load_default_fast(),
  };
  match loaded {
    Ok(library) => json(&LibraryJson {
      master_db_path: library.master_db_path.to_string_lossy().to_string(),
      tracks: library
        .tracks
        .into_iter()
        .map(|t| TrackJson {
          id: t.id,
          title: t.title,
          artist: t.artist,
          album: t.album,
          bpm: t.bpm,
          duration_seconds: t.duration_seconds,
          rating: t.rating,
          tags: t.tags,
          release_date: t.release_date,
          file_path: t.file_path.to_string_lossy().to_string(),
        })
        .collect(),
      playlists: library
        .playlists
        .into_iter()
        .map(|p| PlaylistJson {
          id: p.id,
          name: p.name,
          parent_id: p.parent_id,
          is_folder: p.is_folder,
          track_ids: p.track_ids,
        })
        .collect(),
    }),
    Err(err) => json(&ErrorJson {
      error: err.to_string(),
    }),
  }
}

#[derive(Serialize)]
struct CueJson {
  hot_cue: u32,
  time_ms: u32,
  loop_time_ms: u32,
  is_loop: bool,
  color_rgb: Option<[u8; 3]>,
  comment: Option<String>,
}

#[derive(Serialize)]
struct AnalysisJson {
  /// Beat positions in milliseconds.
  beats_ms: Vec<f32>,
  cues: Vec<CueJson>,
  /// RGB triplets of the 3-band waveform, flattened.
  waveform_rgb: Vec<u8>,
}

/// One track's beat grid, cues and 3-band waveform colours.
#[no_mangle]
pub unsafe extern "C" fn sujay_library_track_analysis_json(
  master_db: *const c_char,
  content_id: *const c_char,
) -> *mut c_char {
  let (Some(master_db), Some(content_id)) = (c_str(master_db), c_str(content_id)) else {
    return json(&ErrorJson {
      error: "missing arguments".into(),
    });
  };
  let analysis = match sujay_library::load_track_analysis_from_master_db(master_db, content_id) {
    Ok(a) => a,
    Err(err) => {
      return json(&ErrorJson {
        error: err.to_string(),
      })
    }
  };
  // The cue reader merges the database's cue table with the ANLZ cues; the
  // analysis alone has only the latter.
  let cues = sujay_library::load_track_cues_from_master_db(master_db, content_id)
    .unwrap_or(analysis.cues.clone());
  json(&AnalysisJson {
    beats_ms: analysis.beats_ms,
    cues: cues
      .into_iter()
      .map(|c| CueJson {
        hot_cue: c.hot_cue,
        time_ms: c.time_ms,
        loop_time_ms: c.loop_time_ms,
        is_loop: c.is_loop,
        color_rgb: c.color_rgb.map(|(r, g, b)| [r, g, b]),
        comment: c.comment,
      })
      .collect(),
    waveform_rgb: analysis
      .waveform
      .iter()
      .flat_map(|s| [s.red, s.green, s.blue])
      .collect(),
  })
}
