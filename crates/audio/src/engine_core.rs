//! NAPI-free audio engine core.
//!
//! This module contains all the DJ audio processing logic with no dependency
//! on Node.js / NAPI.  It can be used directly from:
//!
//!  - The Rust-native host binary (`apps/desktop`)
//!  - The NAPI wrapper (`audio_engine.rs`) which exposes it to Node.js / Electron
//!
//! The public API surface:
//!
//!  * [`AudioEngineCore`] — the main struct
//!  * [`EngineStateUpdate`] — state snapshot emitted at 30 FPS via a callback
//!  * [`EqCutState`] / [`LoopState`] / [`DeviceConfigCore`] — plain data types

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use parking_lot::Mutex;
use soundtouch::{Setting, SoundTouch};
use thread_priority::{set_current_thread_priority, ThreadPriority};

use crate::engine_backend::{EngineIoConfig, EqCutState, RenderInput, WebAudioBackend};
use crate::recorder::RecordingThread;

const DEFAULT_SAMPLE_RATE: u32 = 44_100;
const DEFAULT_CHANNELS: u16 = 2;
const FRAMES_PER_CHUNK: usize = 2048;

// ── Public data types ────────────────────────────────────────────────────────

/// Loop state for a deck.
#[derive(Clone, Copy, Default, Debug)]
pub struct LoopState {
  /// Whether loop is currently enabled.
  pub enabled: bool,
  /// Loop start position in audio frames.
  pub start: f64,
  /// Loop end position in audio frames.
  pub end: f64,
}

/// Device configuration for [`AudioEngineCore::configure_device`].
#[derive(Default, Debug)]
pub struct DeviceConfigCore {
  /// Device name (stable across restarts). `None` → system default.
  pub device_id: Option<String>,
  /// Main output channels `[left, right]`; `-1` = disabled.
  pub main_channels: Option<Vec<i32>>,
  /// Cue output channels `[left, right]`; `-1` = disabled.
  pub cue_channels: Option<Vec<i32>>,
}

/// State snapshot emitted to the callback at ~30 FPS.
#[derive(Clone, Debug, Default)]
pub struct EngineStateUpdate {
  pub deck_a_position: Option<f64>,
  pub deck_b_position: Option<f64>,
  pub deck_a_playing: bool,
  pub deck_b_playing: bool,
  pub crossfader_position: f64,
  pub is_crossfading: bool,
  pub deck_a_peak: f64,
  pub deck_b_peak: f64,
  pub deck_a_peak_hold: f64,
  pub deck_b_peak_hold: f64,
  pub master_tempo: f64,
  pub deck_a_track_id: Option<String>,
  pub deck_b_track_id: Option<String>,
  pub deck_a_gain: f64,
  pub deck_b_gain: f64,
  pub deck_a_cue_enabled: bool,
  pub deck_b_cue_enabled: bool,
  pub deck_a_eq_cut: EqCutState,
  pub deck_b_eq_cut: EqCutState,
  pub deck_a_loop: LoopState,
  pub deck_b_loop: LoopState,
  pub deck_a_total_frames: Option<f64>,
  pub deck_b_total_frames: Option<f64>,
  pub deck_a_bpm: Option<f64>,
  pub deck_b_bpm: Option<f64>,
  pub mic_available: bool,
  pub mic_enabled: bool,
  pub mic_peak: f64,
  /// Whether a recording session is currently active.
  pub is_recording: bool,
  pub update_reason: String,
  pub sample_rate: f64,
}

// ── Private internal structs ─────────────────────────────────────────────────

/// Thin EQ kill-switch wrapper (delegates to engine_backend's EqCutState).
struct EqProcessor {
  cut_state: EqCutState,
}

#[derive(Clone, Copy, Debug)]
enum EqBand { Low, Mid, High }

impl EqProcessor {
  fn new(_max_frames: usize) -> Self {
    Self { cut_state: EqCutState::default() }
  }
  fn set_cut(&mut self, band: EqBand, enabled: bool) {
    match band {
      EqBand::Low  => self.cut_state.low  = enabled,
      EqBand::Mid  => self.cut_state.mid  = enabled,
      EqBand::High => self.cut_state.high = enabled,
    }
  }
  fn get_cut_state(&self) -> EqCutState { self.cut_state }
}

/// SoundTouch-based time stretcher with reservoir.
struct TimeStretcher {
  soundtouch: SoundTouch,
  current_tempo: f32,
  output_buffer: Vec<f32>,
  reservoir: Vec<f32>,
}

impl TimeStretcher {
  fn new(sample_rate: u32, channels: u16) -> Self {
    let mut soundtouch = SoundTouch::new();
    soundtouch
      .set_channels(channels as u32)
      .set_sample_rate(sample_rate)
      .set_tempo(1.0)
      .set_setting(Setting::UseQuickseek, 1);
    Self {
      soundtouch,
      current_tempo: 1.0,
      output_buffer: vec![0.0; FRAMES_PER_CHUNK * channels as usize * 2],
      reservoir: Vec::new(),
    }
  }

  fn process(
    &mut self,
    pcm_data: &[f32],
    position: usize,
    tempo: f32,
    frames_needed: usize,
    output: &mut [f32],
  ) -> usize {
    let channels = DEFAULT_CHANNELS as usize;
    let total_frames = pcm_data.len() / channels;

    if (tempo - self.current_tempo).abs() > 0.001 {
      self.soundtouch.set_tempo(tempo as f64);
      self.current_tempo = tempo;
    }

    let target_reservoir = frames_needed * 2;
    let mut frames_fed = 0;

    while self.reservoir.len() / channels < target_reservoir {
      let remaining = total_frames.saturating_sub(position + frames_fed);
      if remaining == 0 { break; }
      let chunk_size = remaining.min(1024);
      let start_idx = (position + frames_fed) * channels;
      let end_idx = start_idx + chunk_size * channels;
      if end_idx <= pcm_data.len() {
        self.soundtouch.put_samples(&pcm_data[start_idx..end_idx], chunk_size);
        frames_fed += chunk_size;
      }
      self.collect_output();
    }
    self.collect_output();

    let available = self.reservoir.len() / channels;
    let to_copy = available.min(frames_needed);

    if to_copy > 0 {
      let copy_samples = to_copy * channels;
      output[..copy_samples].copy_from_slice(&self.reservoir[..copy_samples]);
      self.reservoir.drain(..copy_samples);
    }

    if to_copy < frames_needed {
      let start = to_copy * channels;
      for sample in &mut output[start..frames_needed * channels] { *sample = 0.0; }
    }

    frames_fed
  }

  fn collect_output(&mut self) {
    let channels = DEFAULT_CHANNELS as usize;
    let buf_frames = self.output_buffer.len() / channels;
    loop {
      let received = self.soundtouch.receive_samples(&mut self.output_buffer, buf_frames);
      if received == 0 { break; }
      self.reservoir.extend_from_slice(&self.output_buffer[..received * channels]);
    }
  }

  fn clear(&mut self) {
    self.soundtouch.clear();
    self.reservoir.clear();
  }
}

struct DeckState {
  pcm_data: Option<Vec<f32>>,
  position: usize,
  playing: bool,
  bpm: Option<f32>,
  rate: f32,
  gain: f32,
  track_id: Option<String>,
  time_stretcher: TimeStretcher,
  eq_processor: EqProcessor,
  loop_enabled: bool,
  loop_start: usize,
  loop_end: usize,
}

impl DeckState {
  fn new(sample_rate: u32) -> Self {
    Self {
      pcm_data: None,
      position: 0,
      playing: false,
      bpm: None,
      rate: 1.0,
      gain: 1.0,
      track_id: None,
      time_stretcher: TimeStretcher::new(sample_rate, DEFAULT_CHANNELS),
      eq_processor: EqProcessor::new(FRAMES_PER_CHUNK),
      loop_enabled: false,
      loop_start: 0,
      loop_end: 0,
    }
  }
}

struct CrossfadeState {
  position: f32,
  active: bool,
  direction: Option<CrossfadeDirection>,
  remaining_frames: usize,
  total_frames: usize,
  start_position: f32,
  target_position: f32,
}

impl Default for CrossfadeState {
  fn default() -> Self {
    Self {
      position: 0.0,
      active: false,
      direction: None,
      remaining_frames: 0,
      total_frames: 0,
      start_position: 0.0,
      target_position: 0.0,
    }
  }
}

#[derive(Clone, Copy, PartialEq)]
enum CrossfadeDirection { AtoB, BtoA }

struct LevelMeterState {
  deck_a_peak: f32,
  deck_b_peak: f32,
  deck_a_peak_hold: f32,
  deck_b_peak_hold: f32,
  deck_a_peak_hold_time: Instant,
  deck_b_peak_hold_time: Instant,
}

impl Default for LevelMeterState {
  fn default() -> Self {
    Self {
      deck_a_peak: 0.0,
      deck_b_peak: 0.0,
      deck_a_peak_hold: 0.0,
      deck_b_peak_hold: 0.0,
      deck_a_peak_hold_time: Instant::now(),
      deck_b_peak_hold_time: Instant::now(),
    }
  }
}

struct ChannelConfig {
  output_channels: u16,
  output_device_name: Option<String>,
  main_channels: [Option<u16>; 2],
  cue_channels: [Option<u16>; 2],
  deck_a_cue: bool,
  deck_b_cue: bool,
}

impl Default for ChannelConfig {
  fn default() -> Self {
    Self {
      output_channels: 2,
      output_device_name: None,
      main_channels: [Some(0), Some(1)],
      cue_channels: [None, None],
      deck_a_cue: false,
      deck_b_cue: false,
    }
  }
}

struct MicrophoneState {
  enabled: bool,
  gain: f32,
  talkover_ducking: f32,
  input_buffer: VecDeque<f32>,
  peak: f32,
}

impl Default for MicrophoneState {
  fn default() -> Self {
    Self {
      enabled: false,
      gain: 1.0,
      talkover_ducking: 0.5,
      input_buffer: VecDeque::new(),
      peak: 0.0,
    }
  }
}

struct EngineState {
  deck_a: DeckState,
  deck_b: DeckState,
  crossfade: CrossfadeState,
  levels: LevelMeterState,
  channel_config: ChannelConfig,
  microphone: MicrophoneState,
  master_tempo: f32,
  running: bool,
  configuring: bool,
  mic_available: bool,
  is_recording: bool,
  update_reason: Option<String>,
}

impl EngineState {
  fn new(sample_rate: u32) -> Self {
    Self {
      deck_a: DeckState::new(sample_rate),
      deck_b: DeckState::new(sample_rate),
      crossfade: CrossfadeState::default(),
      levels: LevelMeterState::default(),
      channel_config: ChannelConfig::default(),
      microphone: MicrophoneState::default(),
      master_tempo: 130.0,
      running: true,
      configuring: false,
      mic_available: false,
      is_recording: false,
      update_reason: None,
    }
  }
}

// ── Core struct ──────────────────────────────────────────────────────────────

/// NAPI-free DJ audio engine.
///
/// Create with [`AudioEngineCore::new`], call [`configure_device`] once to
/// start the output stream, then use the deck/crossfader methods.
pub struct AudioEngineCore {
  state: Arc<Mutex<EngineState>>,
  input_stream: Arc<Mutex<Option<cpal::Stream>>>,
  _process_thread: Option<JoinHandle<()>>,
  recording_thread: Arc<Mutex<Option<RecordingThread>>>,
  pub sample_rate: u32,
}

impl AudioEngineCore {
  /// Create a new engine.
  ///
  /// `callback` is invoked at ~30 FPS from the processing thread with a state
  /// snapshot.  It must be `Send + Sync` because it is called from a background
  /// thread.
  pub fn new(
    sample_rate: Option<u32>,
    callback: Arc<dyn Fn(EngineStateUpdate) + Send + Sync + 'static>,
  ) -> Result<Self, String> {
    let sample_rate = sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE);
    let output_channels = DEFAULT_CHANNELS;

    let state = Arc::new(Mutex::new(EngineState::new(sample_rate)));
    state.lock().channel_config.output_channels = output_channels;

    let recording_thread: Arc<Mutex<Option<RecordingThread>>> =
      Arc::new(Mutex::new(Some(RecordingThread::new())));

    let state_for_process = Arc::clone(&state);
    let recording_thread_for_process = Arc::clone(&recording_thread);

    let sample_rate_for_process = sample_rate;
    let process_thread = thread::spawn(move || {
      let mut backend = WebAudioBackend::new(sample_rate_for_process);
      eprintln!("[AudioEngineCore] Mix/routing backend: web-audio-api");

      match set_current_thread_priority(ThreadPriority::Max) {
        Ok(_)  => eprintln!("[AudioEngineCore] Process thread priority set to Max"),
        Err(e) => eprintln!("[AudioEngineCore] Warning: could not set thread priority: {e:?}"),
      }

      let interval = Duration::from_micros(
        ((FRAMES_PER_CHUNK as f64 / sample_rate_for_process as f64) * 1_000_000.0 * 0.8) as u64,
      );
      let mut last_state_emit = Instant::now();
      let state_emit_interval = Duration::from_millis(33); // ~30 FPS

      loop {
        if !state_for_process.lock().running { break; }

        let current_output_channels = state_for_process.lock().channel_config.output_channels;

        let chunk = {
          let mut state = state_for_process.lock();
          let (chunk, _) = process_audio_chunk_for_backend(
            &mut state,
            sample_rate_for_process,
            current_output_channels,
            &mut backend,
          );
          chunk
        };

        if let Some(ref mut rt) = *recording_thread_for_process.lock() {
          rt.send_audio_data(&chunk);
        }

        if last_state_emit.elapsed() >= state_emit_interval {
          let state_update = {
            let state = state_for_process.lock();
            create_state_update(&state, sample_rate_for_process)
          };
          callback(state_update);
          last_state_emit = Instant::now();
        }

        thread::sleep(interval);
      }
    });

    Ok(Self {
      state,
      input_stream: Arc::new(Mutex::new(None)),
      _process_thread: Some(process_thread),
      recording_thread,
      sample_rate,
    })
  }

  // ── Device ─────────────────────────────────────────────────────────────────

  pub fn configure_device(&self, config: DeviceConfigCore) -> Result<(), String> {
    let device = get_device(config.device_id.as_deref())?;
    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

    let output_channels = device
      .default_output_config()
      .map_err(|e| format!("Device '{}' error: {}", device_name, e))?
      .channels();

    {
      let mut state = self.state.lock();
      state.channel_config.output_channels = output_channels;
      state.channel_config.output_device_name =
        config.device_id.as_ref().map(|_| device_name.clone());

      let clamp_channel = |c: i32| -> Option<u16> {
        if c >= 0 && (c as u16) < output_channels { Some(c as u16) } else { None }
      };

      if let Some(ref main) = config.main_channels {
        state.channel_config.main_channels = [
          main.first().copied().and_then(&clamp_channel),
          main.get(1).copied().and_then(&clamp_channel),
        ];
      } else {
        state.channel_config.main_channels =
          [Some(0), Some(1.min(output_channels.saturating_sub(1)))];
      }

      if let Some(ref cue) = config.cue_channels {
        state.channel_config.cue_channels = [
          cue.first().copied().and_then(&clamp_channel),
          cue.get(1).copied().and_then(&clamp_channel),
        ];
      }
    }

    let new_input_stream = build_input_stream(&device, Arc::clone(&self.state));
    let has_mic = new_input_stream.is_some();
    *self.input_stream.lock() = new_input_stream;

    {
      let mut state = self.state.lock();
      state.configuring = false;
      state.mic_available = has_mic;
      eprintln!(
        "[AudioEngineCore] Device configured: channels={}, sample_rate={}, main={:?}, cue={:?}, mic={}",
        output_channels, self.sample_rate,
        state.channel_config.main_channels, state.channel_config.cue_channels,
        if has_mic { "available" } else { "N/A" },
      );
    }
    Ok(())
  }

  // ── Deck control ───────────────────────────────────────────────────────────

  pub fn load_track(
    &self,
    deck: u32,
    pcm_data: Vec<f32>,
    bpm: Option<f32>,
    track_id: Option<String>,
  ) -> Result<(), String> {
    let mut state = self.state.lock();
    let master_tempo = state.master_tempo;
    let ds = deck_state_mut(&mut state, deck);
    ds.pcm_data = Some(pcm_data);
    ds.position = 0;
    ds.playing = false;
    ds.bpm = bpm;
    ds.rate = calculate_playback_rate(bpm, master_tempo);
    ds.track_id = track_id;
    ds.time_stretcher.clear();
    state.update_reason = Some("load".to_string());
    Ok(())
  }

  pub fn play(&self, deck: u32) -> Result<(), String> {
    let mut state = self.state.lock();
    if deck == 1 {
      if state.deck_a.pcm_data.is_some() { state.deck_a.playing = true; }
    } else if state.deck_b.pcm_data.is_some() {
      state.deck_b.playing = true;
    }
    state.update_reason = Some("play".to_string());
    Ok(())
  }

  pub fn stop(&self, deck: u32) -> Result<(), String> {
    let mut state = self.state.lock();
    if deck == 1 { state.deck_a.playing = false; } else { state.deck_b.playing = false; }
    state.crossfade.active = false;
    state.crossfade.direction = None;
    state.crossfade.remaining_frames = 0;
    state.update_reason = Some("stop".to_string());
    Ok(())
  }

  pub fn seek(&self, deck: u32, position: f64) -> Result<(), String> {
    let position = position.clamp(0.0, 1.0);
    let mut state = self.state.lock();
    let ds = deck_state_mut(&mut state, deck);
    if let Some(ref pcm) = ds.pcm_data {
      let total_frames = pcm.len() / DEFAULT_CHANNELS as usize;
      ds.position = (total_frames as f64 * position) as usize;
      ds.time_stretcher.clear();
    }
    state.update_reason = Some("seek".to_string());
    Ok(())
  }

  pub fn set_crossfader_position(&self, position: f64) -> Result<(), String> {
    self.state.lock().crossfade.position = position.clamp(0.0, 1.0) as f32;
    Ok(())
  }

  pub fn start_crossfade(&self, target_position: Option<f64>, duration: f64) -> Result<(), String> {
    let mut state = self.state.lock();
    let current = state.crossfade.position;
    let target = target_position
      .map(|p| p.clamp(0.0, 1.0) as f32)
      .unwrap_or(if state.deck_a.playing { 1.0 } else { 0.0 });
    let direction = if target > current { CrossfadeDirection::AtoB } else { CrossfadeDirection::BtoA };
    let total_frames = (duration * self.sample_rate as f64) as usize;
    state.crossfade.active = true;
    state.crossfade.direction = Some(direction);
    state.crossfade.remaining_frames = total_frames;
    state.crossfade.total_frames = total_frames;
    state.crossfade.start_position = current;
    state.crossfade.target_position = target;
    Ok(())
  }

  pub fn set_master_tempo(&self, bpm: f64) -> Result<(), String> {
    if bpm <= 0.0 || bpm > 300.0 { return Ok(()); }
    let mut state = self.state.lock();
    state.master_tempo = bpm as f32;
    state.deck_a.rate = calculate_playback_rate(state.deck_a.bpm, state.master_tempo);
    state.deck_b.rate = calculate_playback_rate(state.deck_b.bpm, state.master_tempo);
    Ok(())
  }

  pub fn set_deck_gain(&self, deck: u32, gain: f64) -> Result<(), String> {
    let gain = gain.clamp(0.0, 1.0) as f32;
    let db_gain = if gain == 0.0 { 0.0 } else { gain * gain };
    let mut state = self.state.lock();
    if deck == 1 { state.deck_a.gain = db_gain; } else { state.deck_b.gain = db_gain; }
    Ok(())
  }

  pub fn set_eq_cut(&self, deck: u32, band: &str, enabled: bool) -> Result<(), String> {
    let eq_band = match band {
      "low"  => EqBand::Low,
      "mid"  => EqBand::Mid,
      "high" => EqBand::High,
      _      => return Err(format!("Invalid EQ band: {}", band)),
    };
    let mut state = self.state.lock();
    if deck == 1 { state.deck_a.eq_processor.set_cut(eq_band, enabled); }
    else         { state.deck_b.eq_processor.set_cut(eq_band, enabled); }
    Ok(())
  }

  pub fn get_eq_cut_state(&self, deck: u32) -> EqCutState {
    let state = self.state.lock();
    if deck == 1 { state.deck_a.eq_processor.get_cut_state() }
    else         { state.deck_b.eq_processor.get_cut_state() }
  }

  pub fn set_deck_cue_enabled(&self, deck: u32, enabled: bool) -> Result<(), String> {
    let mut state = self.state.lock();
    if deck == 1 { state.channel_config.deck_a_cue = enabled; }
    else         { state.channel_config.deck_b_cue = enabled; }
    tracing::debug!(
      "[AudioEngineCore] Cue {}: deck_a={}, deck_b={}, cue_channels={:?}",
      if enabled { "enabled" } else { "disabled" },
      state.channel_config.deck_a_cue,
      state.channel_config.deck_b_cue,
      state.channel_config.cue_channels,
    );
    Ok(())
  }

  pub fn set_channel_config(
    &self, main_left: i32, main_right: i32, cue_left: i32, cue_right: i32,
  ) -> Result<(), String> {
    let mut state = self.state.lock();
    state.channel_config.main_channels = [
      if main_left  >= 0 { Some(main_left  as u16) } else { None },
      if main_right >= 0 { Some(main_right as u16) } else { None },
    ];
    state.channel_config.cue_channels = [
      if cue_left   >= 0 { Some(cue_left  as u16) } else { None },
      if cue_right  >= 0 { Some(cue_right as u16) } else { None },
    ];
    let max_channel = [main_left, main_right, cue_left, cue_right]
      .iter().filter(|&&c| c >= 0).max().copied().unwrap_or(1) as u16;
    state.channel_config.output_channels = max_channel + 1;
    Ok(())
  }

  pub fn get_state(&self) -> EngineStateUpdate {
    create_state_update(&self.state.lock(), self.sample_rate)
  }

  pub fn set_mic_enabled(&self, enabled: bool) -> Result<(), String> {
    let mut state = self.state.lock();
    state.microphone.enabled = enabled;
    if !enabled { state.microphone.input_buffer.clear(); state.microphone.peak = 0.0; }
    eprintln!("[AudioEngineCore] Microphone {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
  }

  pub fn set_mic_gain(&self, gain: f64) -> Result<(), String> {
    self.state.lock().microphone.gain = (gain as f32).clamp(0.0, 2.0);
    Ok(())
  }

  pub fn set_talkover_ducking(&self, ducking: f64) -> Result<(), String> {
    self.state.lock().microphone.talkover_ducking = (ducking as f32).clamp(0.0, 1.0);
    Ok(())
  }

  pub fn set_loop(&self, deck: u32, start: f64, end: f64, enabled: bool) -> Result<(), String> {
    let mut state = self.state.lock();
    let ds = deck_state_mut(&mut state, deck);
    if let Some(ref pcm) = ds.pcm_data {
      let total_frames = pcm.len() / DEFAULT_CHANNELS as usize;
      ds.loop_start   = (total_frames as f64 * start.clamp(0.0, 1.0)) as usize;
      ds.loop_end     = (total_frames as f64 * end.clamp(0.0, 1.0)) as usize;
      ds.loop_enabled = enabled && ds.loop_end > ds.loop_start;
    }
    Ok(())
  }

  pub fn set_beat_loop(&self, deck: u32, start_seconds: f64, end_seconds: f64) -> Result<(), String> {
    let mut state = self.state.lock();
    let ds = deck_state_mut(&mut state, deck);
    if let Some(ref pcm) = ds.pcm_data {
      let total_frames = pcm.len() / DEFAULT_CHANNELS as usize;
      let sr = DEFAULT_SAMPLE_RATE as f64;
      let loop_start = (start_seconds * sr) as usize;
      let loop_end   = ((end_seconds * sr) as usize).min(total_frames);
      if loop_end > loop_start {
        ds.loop_start   = loop_start;
        ds.loop_end     = loop_end;
        ds.loop_enabled = true;
        if ds.position >= loop_end || ds.position < loop_start {
          ds.position = loop_start;
          ds.time_stretcher.clear();
        }
      }
    }
    Ok(())
  }

  pub fn clear_loop(&self, deck: u32) -> Result<(), String> {
    let mut state = self.state.lock();
    let ds = deck_state_mut(&mut state, deck);
    ds.loop_enabled = false;
    ds.loop_start   = 0;
    ds.loop_end     = 0;
    Ok(())
  }

  /// Set a beat loop of `beats` beats starting at the current playback position,
  /// or clear the loop when `beats == 0`.  BPM is taken from the loaded track;
  /// if no BPM is available, falls back to 120 BPM.
  pub fn toggle_beat_loop(&self, deck: u32, beats: f32) -> Result<(), String> {
    let mut state = self.state.lock();
    let sr = self.sample_rate as f64;
    let ds = deck_state_mut(&mut state, deck);
    if beats <= 0.0 {
      ds.loop_enabled = false;
      ds.loop_start   = 0;
      ds.loop_end     = 0;
      return Ok(());
    }
    if ds.pcm_data.is_none() { return Ok(()); }
    let total_frames = ds.pcm_data.as_ref().unwrap().len() / DEFAULT_CHANNELS as usize;
    let bpm = ds.bpm.unwrap_or(120.0) as f64;
    let beat_interval_frames = (sr * 60.0 / bpm).round() as usize;
    let loop_start = ds.position;
    let loop_end   = (loop_start + (beat_interval_frames as f32 * beats).round() as usize).min(total_frames);
    if loop_end > loop_start {
      ds.loop_start   = loop_start;
      ds.loop_end     = loop_end;
      ds.loop_enabled = true;
    }
    Ok(())
  }

  // ── Recording ──────────────────────────────────────────────────────────────

  pub fn start_recording(&self, path: String, format: &str) -> Result<(), String> {
    let recording_format = match format {
      "wav" => crate::recorder::RecordingFormat::Wav,
      "ogg" => crate::recorder::RecordingFormat::Ogg,
      _     => return Err(format!("Unsupported recording format: {}", format)),
    };
    if let Some(ref mut rt) = *self.recording_thread.lock() {
      rt.start_recording(path, recording_format).map_err(|e| e.to_string())?;
      self.state.lock().is_recording = true;
    }
    Ok(())
  }

  pub fn stop_recording(&self) -> Result<(), String> {
    if let Some(ref mut rt) = *self.recording_thread.lock() {
      rt.stop().map_err(|e| e.to_string())?;
      self.state.lock().is_recording = false;
    }
    Ok(())
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  pub fn close(&self) {
    *self.input_stream.lock() = None;
    let mut state = self.state.lock();
    state.running        = false;
    state.deck_a.playing = false;
    state.deck_b.playing = false;
  }
}

pub fn list_output_devices() -> Result<Vec<(String, u16)>, String> {
  let host = cpal::default_host();
  let mut devices = Vec::new();
  for dev in host.devices().map_err(|e| e.to_string())? {
    let Ok(name) = dev.name() else { continue; };
    let Ok(config) = dev.default_output_config() else { continue; };
    devices.push((name, config.channels()));
  }
  devices.sort_by(|a, b| a.0.cmp(&b.0));
  Ok(devices)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn deck_state_mut<'a>(state: &'a mut EngineState, deck: u32) -> &'a mut DeckState {
  if deck == 1 { &mut state.deck_a } else { &mut state.deck_b }
}

fn process_audio_chunk_for_backend(
  state: &mut EngineState,
  sample_rate: u32,
  output_channels: u16,
  backend: &mut WebAudioBackend,
) -> (Vec<f32>, EngineStateUpdate) {
  process_audio_chunk_native(state, sample_rate, output_channels, Some(backend))
}

fn get_device(device_id: Option<&str>) -> Result<cpal::Device, String> {
  let host = cpal::default_host();
  if let Some(name) = device_id {
    for dev in host.devices().map_err(|e| e.to_string())? {
      if let Ok(dev_name) = dev.name() {
        if dev_name == name { return Ok(dev); }
      }
    }
    eprintln!("[AudioEngineCore] Device '{}' not found, using default", name);
  }
  host.default_output_device()
    .ok_or_else(|| "No default output device available".to_string())
}

fn build_input_stream(
  device: &cpal::Device,
  state: Arc<Mutex<EngineState>>,
) -> Option<cpal::Stream> {
  let input_config = match device.default_input_config() {
    Ok(c)  => c,
    Err(_) => return None,
  };
  if input_config.sample_format() != SampleFormat::F32 {
    eprintln!("[AudioEngineCore] Input device does not support f32 format");
    return None;
  }
  let input_sample_rate = input_config.sample_rate().0;
  let input_channels    = input_config.channels();
  let state_for_input   = Arc::clone(&state);

  match device.build_input_stream(
    &input_config.into(),
    move |data: &[f32], _| {
      // Avoid startup lock starvation when input callbacks are very frequent
      // on some multi-channel interfaces (e.g. 4ch USB devices).
      let Some(mut state) = state_for_input.try_lock() else {
        return;
      };
      let ch     = input_channels as usize;
      let frames = data.len() / ch;
      for frame in 0..frames {
        let sample = data[frame * ch];
        state.microphone.input_buffer.push_back(sample);
        state.microphone.input_buffer.push_back(sample);
      }
      let max_samples = (input_sample_rate as usize / 10) * 2;
      while state.microphone.input_buffer.len() > max_samples {
        state.microphone.input_buffer.pop_front();
      }
      let mut peak = 0.0f32;
      for frame in 0..frames { peak = peak.max(data[frame * ch].abs()); }
      state.microphone.peak = state.microphone.peak * 0.9 + peak * 0.1;
    },
    move |err| eprintln!("[AudioEngineCore] Input stream error: {err}"),
    None,
  ) {
    Ok(stream) => {
      if stream.play().is_ok() {
        eprintln!("[AudioEngineCore] Microphone input available ({} channels)", input_channels);
        Some(stream)
      } else { None }
    }
    Err(e) => { eprintln!("[AudioEngineCore] Could not create input stream: {e}"); None }
  }
}

fn calculate_playback_rate(track_bpm: Option<f32>, master_tempo: f32) -> f32 {
  match track_bpm {
    Some(bpm) if bpm > 0.0 => (master_tempo / bpm).clamp(0.5, 2.0),
    _ => 1.0,
  }
}

fn process_audio_chunk_native(
  state: &mut EngineState,
  sample_rate: u32,
  output_channels: u16,
  mut backend: Option<&mut WebAudioBackend>,
) -> (Vec<f32>, EngineStateUpdate) {
  let frames   = FRAMES_PER_CHUNK;
  let channels = DEFAULT_CHANNELS as usize;

  let mut buffer_a  = vec![0.0f32; frames * channels];
  let mut buffer_b  = vec![0.0f32; frames * channels];
  let mut mix_buffer = vec![0.0f32; frames * channels];

  // Deck A
  if state.deck_a.playing {
    if let Some(ref pcm) = state.deck_a.pcm_data {
      let total_frames  = pcm.len() / channels;
      let rate          = state.deck_a.rate;
      let frames_consumed = state.deck_a.time_stretcher.process(pcm, state.deck_a.position, rate, frames, &mut buffer_a);
      state.deck_a.position += frames_consumed;
      if state.deck_a.loop_enabled && state.deck_a.position >= state.deck_a.loop_end {
        state.deck_a.position = state.deck_a.loop_start;
        state.deck_a.time_stretcher.clear();
      } else if state.deck_a.position >= total_frames {
        state.deck_a.playing = false;
        state.deck_a.position = 0;
        state.deck_a.time_stretcher.clear();
      }
    }
  }

  // Deck B
  if state.deck_b.playing {
    if let Some(ref pcm) = state.deck_b.pcm_data {
      let total_frames  = pcm.len() / channels;
      let rate          = state.deck_b.rate;
      let frames_consumed = state.deck_b.time_stretcher.process(pcm, state.deck_b.position, rate, frames, &mut buffer_b);
      state.deck_b.position += frames_consumed;
      if state.deck_b.loop_enabled && state.deck_b.position >= state.deck_b.loop_end {
        state.deck_b.position = state.deck_b.loop_start;
        state.deck_b.time_stretcher.clear();
      } else if state.deck_b.position >= total_frames {
        state.deck_b.playing = false;
        state.deck_b.position = 0;
        state.deck_b.time_stretcher.clear();
      }
    }
  }

  // Auto crossfade
  if state.crossfade.active && state.crossfade.remaining_frames > 0 {
    state.crossfade.remaining_frames = state.crossfade.remaining_frames.saturating_sub(frames);
    if state.crossfade.remaining_frames == 0 {
      state.crossfade.position = state.crossfade.target_position;
      if let Some(dir) = state.crossfade.direction {
        match dir {
          CrossfadeDirection::AtoB => { state.deck_a.playing = false; state.deck_b.playing = true; }
          CrossfadeDirection::BtoA => { state.deck_b.playing = false; state.deck_a.playing = true; }
        }
      }
      state.crossfade.active = false;
      state.crossfade.direction = None;
    } else {
      let progress = 1.0 - (state.crossfade.remaining_frames as f32 / state.crossfade.total_frames as f32);
      state.crossfade.position = state.crossfade.start_position
        + (state.crossfade.target_position - state.crossfade.start_position) * progress;
      if let Some(dir) = state.crossfade.direction {
        match dir {
          CrossfadeDirection::AtoB if !state.deck_b.playing => { state.deck_b.playing = true; }
          CrossfadeDirection::BtoA if !state.deck_a.playing => { state.deck_a.playing = true; }
          _ => {}
        }
      }
    }
  }

  let position = state.crossfade.position;
  let gain_a = if state.deck_a.playing { (position * PI / 2.0).cos() } else { 0.0 };
  let gain_b = if state.deck_b.playing { (position * PI / 2.0).sin() } else { 0.0 };
  let deck_a_gain = gain_a * state.deck_a.gain;
  let deck_b_gain = gain_b * state.deck_b.gain;

  state.levels.deck_a_peak = calculate_peak(&buffer_a, frames) * state.deck_a.gain;
  state.levels.deck_b_peak = calculate_peak(&buffer_b, frames) * state.deck_b.gain;
  update_peak_hold(&mut state.levels);

  let output = if let Some(backend) = backend.as_deref_mut() {
    let io = EngineIoConfig {
      output_channels,
      main_channels: state.channel_config.main_channels,
      cue_channels:  state.channel_config.cue_channels,
      output_device_name: state.channel_config.output_device_name.clone(),
    };
    let _ = backend.configure_io(&io);
    let mic_buffer = read_mic_buffer(state, frames);
    let mic_slice  = mic_buffer.as_deref();
    match backend.render(RenderInput {
      deck_a: Some(&buffer_a),
      deck_b: Some(&buffer_b),
      mic: mic_slice,
      frames,
      crossfader_position: position,
      deck_a_playing: state.deck_a.playing,
      deck_b_playing: state.deck_b.playing,
      deck_a_gain: state.deck_a.gain,
      deck_b_gain: state.deck_b.gain,
      deck_a_cue: state.channel_config.deck_a_cue,
      deck_b_cue: state.channel_config.deck_b_cue,
      talkover_ducking: state.microphone.talkover_ducking,
      mic_enabled: state.microphone.enabled,
      mic_gain: state.microphone.gain,
      deck_a_eq: state.deck_a.eq_processor.get_cut_state(),
      deck_b_eq: state.deck_b.eq_processor.get_cut_state(),
    }) {
      Ok(rendered) => {
        state.levels.deck_a_peak = rendered.deck_a_peak;
        state.levels.deck_b_peak = rendered.deck_b_peak;
        state.microphone.peak    = rendered.mic_peak;
        rendered.interleaved
      }
      Err(err) => {
        eprintln!("[AudioEngineCore] Backend render failed, fallback: {}", err);
        for i in 0..(frames * channels) {
          mix_buffer[i] = buffer_a[i] * deck_a_gain + buffer_b[i] * deck_b_gain;
        }
        apply_mic_talkover(state, &mut mix_buffer, frames);
        let needs_map = output_channels as usize != channels
          || state.channel_config.deck_a_cue || state.channel_config.deck_b_cue
          || state.channel_config.cue_channels[0].is_some()
          || state.channel_config.cue_channels[1].is_some();
        if needs_map {
          map_channels(&mix_buffer, frames, output_channels, &state.channel_config, &buffer_a, &buffer_b)
        } else {
          mix_buffer.iter().map(|s| s.clamp(-1.0, 1.0)).collect()
        }
      }
    }
  } else {
    for i in 0..(frames * channels) {
      mix_buffer[i] = buffer_a[i] * deck_a_gain + buffer_b[i] * deck_b_gain;
    }
    apply_mic_talkover(state, &mut mix_buffer, frames);
    let needs_map = output_channels as usize != channels
      || state.channel_config.deck_a_cue || state.channel_config.deck_b_cue
      || state.channel_config.cue_channels[0].is_some()
      || state.channel_config.cue_channels[1].is_some();
    if needs_map {
      map_channels(&mix_buffer, frames, output_channels, &state.channel_config, &buffer_a, &buffer_b)
    } else {
      mix_buffer.iter().map(|s| s.clamp(-1.0, 1.0)).collect()
    }
  };

  let state_update = create_state_update(state, sample_rate);
  state.update_reason = None;
  (output, state_update)
}

fn calculate_peak(buffer: &[f32], frames: usize) -> f32 {
  let channels  = DEFAULT_CHANNELS as usize;
  let available = frames.min(buffer.len() / channels);
  let mut peak  = 0.0f32;
  for i in 0..available {
    for ch in 0..channels { peak = peak.max(buffer[i * channels + ch].abs()); }
  }
  peak
}

fn update_peak_hold(levels: &mut LevelMeterState) {
  const HOLD_DURATION: Duration = Duration::from_millis(1500);
  const DECAY_RATE: f32 = 6.0; // dB/s

  let now = Instant::now();

  for (peak, hold, hold_time) in [
    (&levels.deck_a_peak, &mut levels.deck_a_peak_hold, &mut levels.deck_a_peak_hold_time),
    (&levels.deck_b_peak, &mut levels.deck_b_peak_hold, &mut levels.deck_b_peak_hold_time),
  ] {
    if *peak > *hold {
      *hold = *peak;
      *hold_time = now;
    } else if now.duration_since(*hold_time) > HOLD_DURATION {
      let decay_time = (now.duration_since(*hold_time) - HOLD_DURATION).as_secs_f32();
      let decay_db   = DECAY_RATE * decay_time;
      let current_db = if *hold > 0.0 { 20.0 * hold.log10() } else { f32::NEG_INFINITY };
      let new_db     = current_db - decay_db;
      *hold = if new_db == f32::NEG_INFINITY { 0.0 } else { 10.0f32.powf(new_db / 20.0).max(*peak) };
    }
  }
}

fn apply_mic_talkover(state: &mut EngineState, mix_buffer: &mut [f32], frames: usize) {
  let channels = DEFAULT_CHANNELS as usize;
  let mic = &mut state.microphone;
  if mic.input_buffer.len() < frames * channels { return; }
  let (music_attenuation, mic_gain) = if mic.enabled {
    (1.0 - mic.talkover_ducking, mic.gain)
  } else {
    (1.0, 0.0)
  };
  let mut peak = 0.0f32;
  for i in 0..frames {
    let base = i * channels;
    let mic_left  = mic.input_buffer.pop_front().unwrap_or(0.0);
    let mic_right = if channels > 1 { mic.input_buffer.pop_front().unwrap_or(mic_left) } else { mic_left };
    peak = peak.max(mic_left.abs()).max(mic_right.abs());
    mix_buffer[base] = mix_buffer[base] * music_attenuation + mic_left * mic_gain;
    if channels > 1 { mix_buffer[base + 1] = mix_buffer[base + 1] * music_attenuation + mic_right * mic_gain; }
  }
  mic.peak = peak;
}

fn read_mic_buffer(state: &mut EngineState, frames: usize) -> Option<Vec<f32>> {
  let channels = DEFAULT_CHANNELS as usize;
  let mic = &mut state.microphone;
  if mic.input_buffer.len() < frames * channels { return None; }
  let mut out = vec![0.0f32; frames * channels];
  let mut peak = mic.peak;
  for s in &mut out { let v = mic.input_buffer.pop_front().unwrap_or(0.0); peak = peak.max(v.abs()); *s = v; }
  mic.peak = peak;
  Some(out)
}

fn map_channels(
  mix: &[f32], frames: usize, output_channels: u16,
  config: &ChannelConfig, buffer_a: &[f32], buffer_b: &[f32],
) -> Vec<f32> {
  let channels = DEFAULT_CHANNELS as usize;
  let out_ch   = output_channels as usize;
  let mut output = vec![0.0f32; frames * out_ch];
  let [main_l, main_r] = config.main_channels;
  let [cue_l,  cue_r ] = config.cue_channels;

  for frame in 0..frames {
    let mix_base = frame * channels;
    let out_base = frame * out_ch;
    let main_left  = mix[mix_base];
    let main_right = mix.get(mix_base + 1).copied().unwrap_or(main_left);
    let mono_main  = (main_left + main_right) * 0.5;

    if let (Some(l), Some(r)) = (main_l, main_r) {
      output[out_base + l as usize] = main_left;
      output[out_base + r as usize] = main_right;
    } else if let Some(l) = main_l { output[out_base + l as usize] = mono_main; }
    else if let Some(r) = main_r   { output[out_base + r as usize] = mono_main; }

    let cue_enabled = config.deck_a_cue || config.deck_b_cue;
    if cue_enabled && (cue_l.is_some() || cue_r.is_some()) {
      let (mut cue_left, mut cue_right, mut cue_sources) = (0.0f32, 0.0f32, 0u32);
      if config.deck_a_cue {
        cue_left  += buffer_a[mix_base];
        cue_right += buffer_a.get(mix_base + 1).copied().unwrap_or(buffer_a[mix_base]);
        cue_sources += 1;
      }
      if config.deck_b_cue {
        cue_left  += buffer_b[mix_base];
        cue_right += buffer_b.get(mix_base + 1).copied().unwrap_or(buffer_b[mix_base]);
        cue_sources += 1;
      }
      if cue_sources > 0 {
        let norm = 1.0 / cue_sources as f32;
        cue_left  = (cue_left  * norm).clamp(-1.0, 1.0);
        cue_right = (cue_right * norm).clamp(-1.0, 1.0);
        let mono_cue = (cue_left + cue_right) * 0.5;
        if let (Some(l), Some(r)) = (cue_l, cue_r) {
          output[out_base + l as usize] = cue_left;
          output[out_base + r as usize] = cue_right;
        } else if let Some(l) = cue_l { output[out_base + l as usize] = mono_cue; }
        else if let Some(r) = cue_r   { output[out_base + r as usize] = mono_cue; }
      }
    }
  }

  output.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
  output
}

fn create_state_update(state: &EngineState, sample_rate: u32) -> EngineStateUpdate {
  let deck_a_position = state.deck_a.pcm_data.as_ref().map(|_| state.deck_a.position as f64);
  let deck_b_position = state.deck_b.pcm_data.as_ref().map(|_| state.deck_b.position as f64);
  let deck_a_total_frames = state.deck_a.pcm_data.as_ref().map(|p| (p.len() / DEFAULT_CHANNELS as usize) as f64);
  let deck_b_total_frames = state.deck_b.pcm_data.as_ref().map(|p| (p.len() / DEFAULT_CHANNELS as usize) as f64);
  let update_reason = state.update_reason.clone().unwrap_or_else(|| "periodic".to_string());
  let deck_a_eq = state.deck_a.eq_processor.get_cut_state();
  let deck_b_eq = state.deck_b.eq_processor.get_cut_state();

  EngineStateUpdate {
    deck_a_position,
    deck_b_position,
    deck_a_playing:     state.deck_a.playing,
    deck_b_playing:     state.deck_b.playing,
    crossfader_position: state.crossfade.position as f64,
    is_crossfading:     state.crossfade.active,
    deck_a_peak:        state.levels.deck_a_peak as f64,
    deck_b_peak:        state.levels.deck_b_peak as f64,
    deck_a_peak_hold:   state.levels.deck_a_peak_hold as f64,
    deck_b_peak_hold:   state.levels.deck_b_peak_hold as f64,
    master_tempo:       state.master_tempo as f64,
    deck_a_track_id:    state.deck_a.track_id.clone(),
    deck_b_track_id:    state.deck_b.track_id.clone(),
    deck_a_gain:        state.deck_a.gain as f64,
    deck_b_gain:        state.deck_b.gain as f64,
    deck_a_cue_enabled: state.channel_config.deck_a_cue,
    deck_b_cue_enabled: state.channel_config.deck_b_cue,
    deck_a_eq_cut:      deck_a_eq,
    deck_b_eq_cut:      deck_b_eq,
    deck_a_loop: LoopState {
      enabled: state.deck_a.loop_enabled,
      start:   state.deck_a.loop_start as f64,
      end:     state.deck_a.loop_end   as f64,
    },
    deck_b_loop: LoopState {
      enabled: state.deck_b.loop_enabled,
      start:   state.deck_b.loop_start as f64,
      end:     state.deck_b.loop_end   as f64,
    },
    deck_a_total_frames,
    deck_b_total_frames,
    deck_a_bpm: state.deck_a.bpm.map(|b| b as f64),
    deck_b_bpm: state.deck_b.bpm.map(|b| b as f64),
    mic_available: state.mic_available,
    mic_enabled:   state.microphone.enabled,
    mic_peak:      state.microphone.peak as f64,
    is_recording:  state.is_recording,
    update_reason,
    sample_rate:   sample_rate as f64,
  }
}
