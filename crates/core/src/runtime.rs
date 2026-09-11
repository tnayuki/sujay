//! The host runtime: owns the audio engine, the preferences, the background
//! decode and library loads and the per-deck buffers, and turns engine state
//! into what a UI shows. No windowing, no rendering — a host calls `tick` at
//! its own cadence and pushes whatever the returned `Tick` says changed.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sujay_audio::engine_core::{AudioEngineCore, EngineStateUpdate};

use crate::decode::{spawn_decode, DecodeReady, DecodeResult};
use crate::library::{
  normalize_track_path, spawn_rekordbox_library_load, LibraryLoadResult, RekordboxTrackOverride,
};
use crate::preferences::{available_audio_devices, settings_file_path, AppPreferences};
use crate::state::{
  AudioDeviceInfo, ConsoleVisualState, DeckConsoleVisualState, DeckCueVisualState,
  LibraryVisualState, PreferencesState, TitlebarState,
};

/// Deck identifier as the UI uses it: 1 = A, 2 = B.
pub type DeckId = u8;

fn deck_index(deck: DeckId) -> usize {
  if deck <= 1 {
    0
  } else {
    1
  }
}

/// Per-deck buffers the UI draws from. All positions are audio frame indices.
#[derive(Clone, Debug, Default)]
pub struct DeckBuffers {
  pub waveform: Vec<f32>,
  pub waveform_colors: Vec<[u8; 3]>,
  pub beats: Vec<f32>,
  pub intro: Option<f32>,
  pub outro: Option<f32>,
  /// `(position_frames, total_frames, sample_rate)` once a track is loaded.
  pub progress: Option<(f32, f32, f32)>,
}

/// What one `tick` changed, so a host pushes only that to its renderer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tick {
  pub console: bool,
  pub progress: [bool; 2],
  pub waveform: [bool; 2],
  pub markers: [bool; 2],
  pub library: bool,
  pub preferences: bool,
  /// A decoded track is waiting for the engine to accept it; tick again within ~1 ms.
  pub retry_soon: bool,
}

impl Tick {
  pub fn any(&self) -> bool {
    self.console
      || self.progress.iter().any(|v| *v)
      || self.waveform.iter().any(|v| *v)
      || self.markers.iter().any(|v| *v)
      || self.library
      || self.preferences
  }
}

pub struct Core {
  engine: Option<Arc<AudioEngineCore>>,
  /// Latest state update from the audio engine (shared with the audio callback).
  last_state: Arc<Mutex<Option<EngineStateUpdate>>>,
  decode_tx: Sender<DecodeResult>,
  decode_rx: Receiver<DecodeResult>,
  /// Completed decode waiting for a non-blocking audio-engine handoff.
  pending_decode: Option<DecodeResult>,
  sys: sysinfo::System,
  /// Last whole-second timestamp used for titlebar system-info refresh.
  last_titlebar_second: Option<u64>,
  /// Titlebar system fields that only need 1 Hz refresh.
  cached_titlebar: TitlebarState,
  console: ConsoleVisualState,
  console_initialised: bool,
  decks: [DeckBuffers; 2],
  deck_cues: [Vec<DeckCueVisualState>; 2],
  /// When the current recording session started (None = not recording).
  rec_started_at: Option<Instant>,
  settings_path: PathBuf,
  preferences: AppPreferences,
  audio_devices: Vec<AudioDeviceInfo>,
  preferences_dirty: bool,
  library: LibraryVisualState,
  library_dirty: bool,
  rekordbox_track_overrides: HashMap<PathBuf, RekordboxTrackOverride>,
  rekordbox_track_ids: HashMap<PathBuf, String>,
  rekordbox_master_db_path: Option<PathBuf>,
  rekordbox_load_tx: Sender<LibraryLoadResult>,
  rekordbox_load_rx: Receiver<LibraryLoadResult>,
  rekordbox_load_in_flight: bool,
  rekordbox_db_modified: Option<SystemTime>,
  next_rekordbox_reload_check: Instant,
}

impl Default for Core {
  fn default() -> Self {
    Self::new()
  }
}

impl Core {
  pub fn new() -> Self {
    let (decode_tx, decode_rx) = mpsc::channel();
    let (rekordbox_load_tx, rekordbox_load_rx) = mpsc::channel();
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);
    let settings_path = settings_file_path();
    let preferences = AppPreferences::load(&settings_path);
    Self {
      engine: None,
      last_state: Arc::new(Mutex::new(None)),
      decode_tx,
      decode_rx,
      pending_decode: None,
      sys,
      last_titlebar_second: None,
      cached_titlebar: TitlebarState::default(),
      console: ConsoleVisualState::default(),
      console_initialised: false,
      decks: [DeckBuffers::default(), DeckBuffers::default()],
      deck_cues: [Vec::new(), Vec::new()],
      rec_started_at: None,
      settings_path,
      preferences,
      audio_devices: vec![],
      preferences_dirty: false,
      library: LibraryVisualState::default(),
      library_dirty: false,
      rekordbox_track_overrides: HashMap::new(),
      rekordbox_track_ids: HashMap::new(),
      rekordbox_master_db_path: None,
      rekordbox_load_tx,
      rekordbox_load_rx,
      rekordbox_load_in_flight: false,
      rekordbox_db_modified: None,
      next_rekordbox_reload_check: Instant::now(),
    }
  }

  /// Start the audio engine, apply the saved device configuration and begin
  /// loading the rekordbox library in the background.
  pub fn start(&mut self) -> Result<(), String> {
    let last_state = Arc::clone(&self.last_state);
    let engine = Arc::new(
      AudioEngineCore::new(
        Some(44100),
        Arc::new(move |state: EngineStateUpdate| {
          if let Ok(mut guard) = last_state.lock() {
            *guard = Some(state);
          }
        }),
      )
      .map_err(|err| err.to_string())?,
    );

    self.audio_devices = available_audio_devices();
    self.preferences.normalize(&self.audio_devices);
    if let Err(err) = engine.configure_device(self.preferences.device_config()) {
      tracing::warn!("failed to configure initial audio device: {}", err);
    }
    self.preferences_dirty = true;
    self.engine = Some(engine);

    self.start_rekordbox_library_load();
    Ok(())
  }

  pub fn shutdown(&mut self) {
    if let Some(engine) = self.engine.take() {
      engine.close();
    }
  }

  // ── State the UI reads ───────────────────────────────────────────────────

  pub fn console_state(&self) -> &ConsoleVisualState {
    &self.console
  }

  pub fn preferences_state(&self) -> PreferencesState {
    self.preferences.to_state(&self.audio_devices)
  }

  pub fn library_state(&self) -> &LibraryVisualState {
    &self.library
  }

  pub fn deck(&self, deck: DeckId) -> &DeckBuffers {
    &self.decks[deck_index(deck)]
  }

  // ── Commands ─────────────────────────────────────────────────────────────

  pub fn play(&self, deck: DeckId) {
    if let Some(engine) = &self.engine {
      let _ = engine.play(deck as u32);
    }
  }

  pub fn stop(&self, deck: DeckId) {
    if let Some(engine) = &self.engine {
      let _ = engine.stop(deck as u32);
    }
  }

  pub fn set_crossfader(&self, position: f32) {
    if let Some(engine) = &self.engine {
      let _ = engine.set_crossfader_position(position as f64);
    }
  }

  pub fn set_master_tempo(&self, bpm: f32) {
    if let Some(engine) = &self.engine {
      let _ = engine.set_master_tempo(bpm as f64);
    }
  }

  pub fn set_deck_gain(&self, deck: DeckId, gain: f32) {
    if let Some(engine) = &self.engine {
      let _ = engine.set_deck_gain(deck as u32, gain as f64);
    }
  }

  pub fn set_cue(&self, deck: DeckId, enabled: bool) {
    if let Some(engine) = &self.engine {
      let _ = engine.set_deck_cue_enabled(deck as u32, enabled);
    }
  }

  pub fn set_eq(&self, deck: DeckId, band: &str, kill: bool) {
    if let Some(engine) = &self.engine {
      let _ = engine.set_eq_cut(deck as u32, band, kill);
    }
  }

  /// `position` is a fraction of the track (0..1).
  pub fn seek(&self, deck: DeckId, position: f32) {
    if let Some(engine) = &self.engine {
      let _ = engine.seek(deck as u32, position as f64);
    }
  }

  /// Jump to a cue; a loop cue also arms its loop, a plain cue clears any loop.
  /// Positions are fractions of the track (0..1).
  pub fn recall_cue(&self, deck: DeckId, position: f32, loop_end: Option<f32>) {
    let Some(engine) = &self.engine else {
      return;
    };
    let _ = engine.seek(deck as u32, position as f64);
    if let Some(loop_end) = loop_end {
      let _ = engine.set_loop(deck as u32, position as f64, loop_end as f64, true);
    } else {
      let _ = engine.clear_loop(deck as u32);
    }
  }

  /// Set a beat loop of `beats` from the beat before the current position, or
  /// clear the loop when `beats <= 0`. Falls back to a 120 BPM interval past the
  /// end of the grid or when there is no grid at all.
  pub fn toggle_loop(&self, deck: DeckId, beats: f32) {
    let Some(engine) = &self.engine else {
      return;
    };
    if beats <= 0.0 {
      let _ = engine.clear_loop(deck as u32);
      return;
    }
    let buffers = &self.decks[deck_index(deck)];
    let beat_grid = &buffers.beats;
    let current_pos = buffers.progress.map(|(pos, _, _)| pos).unwrap_or(0.0);

    let start_beat_idx = beat_grid
      .partition_point(|&b| b <= current_pos)
      .saturating_sub(1);
    let start_frames = beat_grid
      .get(start_beat_idx)
      .copied()
      .unwrap_or(current_pos);
    let beats_whole = beats.floor() as usize;
    let beats_frac = beats - beats.floor();
    let end_frames = if beats_frac < 0.001 {
      let end_idx = start_beat_idx + beats_whole;
      if end_idx < beat_grid.len() {
        beat_grid[end_idx]
      } else {
        let beat_interval = if beat_grid.len() >= 2 {
          beat_grid[beat_grid.len() - 1] - beat_grid[beat_grid.len() - 2]
        } else {
          engine.sample_rate as f32 * 60.0 / 120.0
        };
        start_frames + beat_interval * beats_whole as f32
      }
    } else {
      let beat_interval = if start_beat_idx + 1 < beat_grid.len() {
        beat_grid[start_beat_idx + 1] - start_frames
      } else if beat_grid.len() >= 2 {
        beat_grid[beat_grid.len() - 1] - beat_grid[beat_grid.len() - 2]
      } else {
        engine.sample_rate as f32 * 60.0 / 120.0
      };
      start_frames + beat_interval * beats
    };

    let sr = engine.sample_rate as f64;
    let _ = engine.set_beat_loop(
      deck as u32,
      start_frames as f64 / sr,
      end_frames as f64 / sr,
    );
  }

  pub fn set_mic_enabled(&self, enabled: bool) {
    if let Some(engine) = &self.engine {
      let _ = engine.set_mic_enabled(enabled);
    }
  }

  pub fn start_recording(&self) {
    let Some(engine) = &self.engine else {
      return;
    };
    match self.preferences.prepare_recording_path() {
      Ok(path) => {
        if let Err(err) = engine.start_recording(
          path.to_string_lossy().to_string(),
          &self.preferences.recording_format,
        ) {
          tracing::warn!("failed to start recording: {}", err);
        }
      }
      Err(err) => {
        tracing::warn!("failed to prepare recording path: {}", err);
      }
    }
  }

  pub fn stop_recording(&self) {
    if let Some(engine) = &self.engine {
      let _ = engine.stop_recording();
    }
  }

  /// Decode `path` in the background and load it into `deck` when ready,
  /// joining rekordbox metadata by path when the library knows the file.
  pub fn load_file(&self, deck: DeckId, path: PathBuf) {
    let (override_meta, content_id) = if let (Some(ov), Some(id)) = (
      self.rekordbox_track_overrides.get(&path),
      self.rekordbox_track_ids.get(&path),
    ) {
      (Some(ov.clone()), Some(id.clone()))
    } else {
      let key = normalize_track_path(&path);
      (
        self.rekordbox_track_overrides.get(&key).cloned(),
        self.rekordbox_track_ids.get(&key).cloned(),
      )
    };
    eprintln!(
      "[Library] load deck={} path={} override={} content_id={}",
      deck,
      path.display(),
      override_meta.is_some(),
      content_id.as_deref().unwrap_or("-"),
    );
    spawn_decode(
      deck,
      path,
      self.decode_tx.clone(),
      override_meta,
      self.rekordbox_master_db_path.clone(),
      content_id,
    );
  }

  /// Re-enumerate audio devices and re-normalise preferences against them —
  /// what a settings dialog wants right before it opens.
  pub fn refresh_audio_devices(&mut self) {
    self.audio_devices = available_audio_devices();
    self.preferences.normalize(&self.audio_devices);
  }

  /// Persist an edited preferences state and apply it to the engine.
  pub fn apply_preferences(&mut self, state: PreferencesState) {
    self.preferences.apply_state(state, &self.audio_devices);

    if let Err(err) = self.preferences.save(&self.settings_path) {
      tracing::warn!("failed to save preferences: {}", err);
    }
    if let Some(engine) = &self.engine {
      if let Err(err) = engine.configure_device(self.preferences.device_config()) {
        tracing::warn!("failed to apply audio preferences: {}", err);
      }
    }
    self.preferences_dirty = true;
  }

  // ── Per-tick work ────────────────────────────────────────────────────────

  /// Drain background results and the latest engine state. Call at the host's
  /// frame cadence (the engine emits state at ~30 Hz).
  pub fn tick(&mut self) -> Tick {
    let mut tick = Tick {
      preferences: std::mem::take(&mut self.preferences_dirty),
      ..Tick::default()
    };

    self.drain_library_loads(&mut tick);
    self.poll_rekordbox_library_reload();
    tick.library |= std::mem::take(&mut self.library_dirty);

    self.apply_engine_state(&mut tick);
    self.drain_decodes(&mut tick);
    tick
  }

  fn start_rekordbox_library_load(&mut self) {
    if self.rekordbox_load_in_flight {
      return;
    }
    self.rekordbox_load_in_flight = true;

    self.library = LibraryVisualState {
      source_label: "Loading Rekordbox library...".to_owned(),
      tracks: vec![],
      playlists: vec![],
    };
    self.library_dirty = true;

    spawn_rekordbox_library_load(self.rekordbox_load_tx.clone());
  }

  fn poll_rekordbox_library_reload(&mut self) {
    if self.rekordbox_load_in_flight || Instant::now() < self.next_rekordbox_reload_check {
      return;
    }
    self.next_rekordbox_reload_check = Instant::now() + Duration::from_secs(2);

    let Some(master_db_path) = self.rekordbox_master_db_path.as_ref() else {
      return;
    };
    let modified = fs::metadata(master_db_path)
      .and_then(|metadata| metadata.modified())
      .ok();
    if modified.is_some() && modified != self.rekordbox_db_modified {
      eprintln!(
        "[Rekordbox] master.db changed; refreshing library from {}",
        master_db_path.display()
      );
      self.rekordbox_load_in_flight = true;
      spawn_rekordbox_library_load(self.rekordbox_load_tx.clone());
    }
  }

  fn drain_library_loads(&mut self, tick: &mut Tick) {
    while let Ok(result) = self.rekordbox_load_rx.try_recv() {
      let had_library = self.rekordbox_master_db_path.is_some();
      self.rekordbox_load_in_flight = false;
      match result {
        Ok(ready) => {
          eprintln!(
            "[Rekordbox] loaded library: tracks={} source={}",
            ready.tracks.len(),
            ready.source_label
          );
          for track in ready.tracks.iter().take(5) {
            eprintln!(
              "[Rekordbox] track title={:?} artist={:?} path={}",
              track.title, track.artist, track.file_path
            );
          }
          self.rekordbox_db_modified = fs::metadata(&ready.master_db_path)
            .and_then(|metadata| metadata.modified())
            .ok();
          self.rekordbox_master_db_path = Some(ready.master_db_path);
          self.rekordbox_track_overrides = ready.overrides;
          self.rekordbox_track_ids = ready.track_ids;
          self.library = LibraryVisualState {
            source_label: ready.source_label,
            tracks: ready.tracks,
            playlists: ready.playlists,
          };
        }
        Err(err) => {
          tracing::warn!("failed to load rekordbox library: {}", err);
          if !had_library {
            self.rekordbox_master_db_path = None;
            self.rekordbox_track_overrides.clear();
            self.rekordbox_track_ids.clear();
            self.library = LibraryVisualState {
              source_label: "Rekordbox library not found".to_owned(),
              tracks: vec![],
              playlists: vec![],
            };
          }
        }
      }
      tick.library = true;
    }
  }

  fn apply_engine_state(&mut self, tick: &mut Tick) {
    let Some(state) = self
      .last_state
      .lock()
      .ok()
      .and_then(|mut guard| guard.take())
    else {
      return;
    };

    // Track recording start time
    if state.is_recording && self.rec_started_at.is_none() {
      self.rec_started_at = Some(Instant::now());
    } else if !state.is_recording {
      self.rec_started_at = None;
    }

    let now_secs = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    if self.last_titlebar_second != Some(now_secs) {
      self.last_titlebar_second = Some(now_secs);
      self.sys.refresh_cpu_all();
      self.sys.refresh_memory();
      let pid = sysinfo::Pid::from_u32(std::process::id());
      self
        .sys
        .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

      self.cached_titlebar.time_text = local_clock_text(now_secs);
      self.cached_titlebar.cpu_percent = self.sys.global_cpu_usage();
      self.cached_titlebar.mem_mb = self
        .sys
        .process(pid)
        .map(|p| p.memory() / 1024 / 1024)
        .unwrap_or(0);
    }

    let rec_elapsed_secs = self
      .rec_started_at
      .map(|t| t.elapsed().as_secs() as u32)
      .unwrap_or(0);

    let mut cv = engine_state_to_console_visual(&state);
    cv.deck_a.rekordbox_cues = self.deck_cues[0].clone();
    cv.deck_b.rekordbox_cues = self.deck_cues[1].clone();
    cv.titlebar = TitlebarState {
      time_text: self.cached_titlebar.time_text.clone(),
      cpu_percent: self.cached_titlebar.cpu_percent,
      mem_mb: self.cached_titlebar.mem_mb,
      mic_available: state.mic_available,
      mic_enabled: state.mic_enabled,
      mic_peak: state.mic_peak as f32,
      is_recording: state.is_recording,
      rec_elapsed_secs,
    };
    if !self.console_initialised || self.console != cv {
      self.console_initialised = true;
      self.console = cv;
      tick.console = true;
    }

    let sr = state.sample_rate as f32;
    let positions = [
      (state.deck_a_position, state.deck_a_total_frames),
      (state.deck_b_position, state.deck_b_total_frames),
    ];
    for (idx, (position, total)) in positions.into_iter().enumerate() {
      let Some(pos) = position else {
        continue;
      };
      let next = (pos as f32, total.unwrap_or(0.0) as f32, sr);
      if self.decks[idx].progress != Some(next) {
        self.decks[idx].progress = Some(next);
        tick.progress[idx] = true;
      }
    }
  }

  fn drain_decodes(&mut self, tick: &mut Tick) {
    let Some(engine) = self.engine.clone() else {
      return;
    };
    // One result per tick keeps the host loop responsive when several decodes
    // complete together.
    let Some(result) = self
      .pending_decode
      .take()
      .or_else(|| self.decode_rx.try_recv().ok())
    else {
      return;
    };
    match result {
      DecodeResult::Ready(ready) => {
        let DecodeReady {
          deck,
          pcm,
          waveform,
          waveform_colors,
          bpm,
          title,
          beats,
          intro,
          outro,
          cues,
          total_frames,
        } = ready;
        eprintln!(
          "[D&D] Loading deck {} title={:?} bpm={:?}",
          deck, title, bpm
        );
        let mut pcm = Some(pcm);
        let mut track_id = Some(title.clone());
        if !engine.try_load_track(deck as u32, &mut pcm, bpm, beats.clone(), &mut track_id) {
          self.pending_decode = Some(DecodeResult::Ready(DecodeReady {
            deck,
            pcm: pcm.expect("PCM retained when audio engine is busy"),
            waveform,
            waveform_colors,
            bpm,
            title,
            beats,
            intro,
            outro,
            cues,
            total_frames,
          }));
          tick.retry_soon = true;
          return;
        }
        let sr = engine.sample_rate as f32;
        let idx = deck_index(deck);
        self.deck_cues[idx] = cues;
        if let (Some(fb), Some(lb)) = (beats.first().copied(), beats.last().copied()) {
          eprintln!(
            "[DEBUG] deck={} sr={} total_frames={} beats[0]={:.0} beats[-1]={:.0} \
             beat[0]_sec={:.2} beat[-1]_sec={:.2}",
            deck,
            sr,
            total_frames,
            fb,
            lb,
            fb / sr,
            lb / sr,
          );
        }
        let buffers = &mut self.decks[idx];
        buffers.waveform = waveform;
        buffers.waveform_colors = waveform_colors;
        buffers.beats = beats.into_iter().filter(|v| v.is_finite()).collect();
        buffers.intro = intro.filter(|v| v.is_finite());
        buffers.outro = outro.filter(|v| v.is_finite());
        buffers.progress = Some((0.0, total_frames, sr));
        tick.waveform[idx] = true;
        tick.markers[idx] = true;
        tick.progress[idx] = true;
      }
      DecodeResult::Failed(failure) => {
        eprintln!(
          "[D&D] Decode failed for deck={} path={} reason={}",
          failure.deck,
          failure.path.display(),
          failure.error
        );
      }
    }
  }
}

/// "HH:MM:SS" in local time.
fn local_clock_text(now_secs: u64) -> String {
  let ts = now_secs as libc::time_t;
  let mut tm: libc::tm = unsafe { std::mem::zeroed() };
  unsafe {
    libc::localtime_r(&ts, &mut tm);
  }
  format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

// ── Engine state → UI visual state mapping ───────────────────────────────────

fn engine_state_to_console_visual(s: &EngineStateUpdate) -> ConsoleVisualState {
  fn fmt_time(frames: f64, sr: f64) -> String {
    if sr == 0.0 {
      return "0:00".into();
    }
    let secs = (frames / sr) as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
  }

  let sr = s.sample_rate;

  // Compute loop_beats from loop length and track BPM so the correct button
  // appears highlighted.  Falls back to 0.0 (no active button) if unknown.
  let calc_loop_beats = |loop_enabled: bool, start: f64, end: f64, bpm: Option<f64>| -> f32 {
    if !loop_enabled || bpm.is_none() || bpm == Some(0.0) {
      return 0.0;
    }
    let beat_interval = sr * 60.0 / bpm.unwrap();
    let beats = (end - start) / beat_interval;
    // Round to nearest standard value
    let standards = [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    standards
      .iter()
      .copied()
      .min_by(|a, b| {
        (a - beats as f32)
          .abs()
          .partial_cmp(&(b - beats as f32).abs())
          .unwrap()
      })
      .unwrap_or(0.0)
  };

  let deck_a = DeckConsoleVisualState {
    title: s.deck_a_track_id.clone().unwrap_or_else(|| "---".into()),
    time_text: s
      .deck_a_position
      .map(|p| fmt_time(p, sr))
      .unwrap_or_else(|| "0:00".into()),
    bpm_text: s
      .deck_a_bpm
      .map(|b| format!("{:.1}", b))
      .unwrap_or_else(|| "--.-".into()),
    bpm: s.deck_a_bpm.unwrap_or(s.master_tempo) as f32,
    playing: s.deck_a_playing,
    loop_enabled: s.deck_a_loop.enabled,
    loop_beats: calc_loop_beats(
      s.deck_a_loop.enabled,
      s.deck_a_loop.start,
      s.deck_a_loop.end,
      s.deck_a_bpm,
    ),
    loop_start: s.deck_a_loop.start as f32,
    loop_end: s.deck_a_loop.end as f32,
    cue_enabled: s.deck_a_cue_enabled,
    eq_low: s.deck_a_eq_cut.low,
    eq_mid: s.deck_a_eq_cut.mid,
    eq_high: s.deck_a_eq_cut.high,
    gain: s.deck_a_gain as f32,
    peak: s.deck_a_peak as f32,
    rekordbox_cues: Vec::new(),
  };
  let deck_b = DeckConsoleVisualState {
    title: s.deck_b_track_id.clone().unwrap_or_else(|| "---".into()),
    time_text: s
      .deck_b_position
      .map(|p| fmt_time(p, sr))
      .unwrap_or_else(|| "0:00".into()),
    bpm_text: s
      .deck_b_bpm
      .map(|b| format!("{:.1}", b))
      .unwrap_or_else(|| "--.-".into()),
    bpm: s.deck_b_bpm.unwrap_or(s.master_tempo) as f32,
    playing: s.deck_b_playing,
    loop_enabled: s.deck_b_loop.enabled,
    loop_beats: calc_loop_beats(
      s.deck_b_loop.enabled,
      s.deck_b_loop.start,
      s.deck_b_loop.end,
      s.deck_b_bpm,
    ),
    loop_start: s.deck_b_loop.start as f32,
    loop_end: s.deck_b_loop.end as f32,
    cue_enabled: s.deck_b_cue_enabled,
    eq_low: s.deck_b_eq_cut.low,
    eq_mid: s.deck_b_eq_cut.mid,
    eq_high: s.deck_b_eq_cut.high,
    gain: s.deck_b_gain as f32,
    peak: s.deck_b_peak as f32,
    rekordbox_cues: Vec::new(),
  };
  ConsoleVisualState {
    titlebar: TitlebarState::default(), // overwritten by the caller with live data
    deck_a,
    deck_b,
    master_tempo: s.master_tempo as f32,
    crossfader: s.crossfader_position as f32,
  }
}
