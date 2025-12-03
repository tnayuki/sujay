//! Audio Engine - Core DJ mixing engine with dual decks and crossfader
//!
//! This module provides the main audio processing engine that handles:
//! - Dual deck playback with independent positions
//! - Crossfader with Pioneer-style constant power curve
//! - Auto crossfade with configurable duration
//! - Level metering with peak hold
//! - Channel routing for main and cue outputs
//! - Time stretching with pitch preservation (SoundTouch)
//! - 3-band EQ with kill switches
//! - Microphone input with talkover (ducking)

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;
use parking_lot::Mutex;
use soundtouch::{Setting, SoundTouch};
use thread_priority::{set_current_thread_priority, ThreadPriority};

use crate::eq_processor::{EqBand, EqProcessor};

const DEFAULT_SAMPLE_RATE: u32 = 44_100;
const DEFAULT_CHANNELS: u16 = 2;
const FRAMES_PER_CHUNK: usize = 2048;

/// Time stretcher wrapper for pitch-preserved tempo adjustment
struct TimeStretcher {
  soundtouch: SoundTouch,
  current_tempo: f32,
  output_buffer: Vec<f32>,
  /// Internal reservoir of output frames from previous calls
  reservoir: Vec<f32>,
}

impl TimeStretcher {
  fn new(sample_rate: u32, channels: u16) -> Self {
    let mut soundtouch = SoundTouch::new();
    soundtouch
      .set_channels(channels as u32)
      .set_sample_rate(sample_rate)
      .set_tempo(1.0)
      // Use quickseek for better performance
      .set_setting(Setting::UseQuickseek, 1);

    Self {
      soundtouch,
      current_tempo: 1.0,
      output_buffer: vec![0.0; FRAMES_PER_CHUNK * channels as usize * 2],
      reservoir: Vec::new(),
    }
  }

  /// Process PCM data with time stretching
  /// Returns the number of input frames consumed
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

    // Update tempo if changed
    if (tempo - self.current_tempo).abs() > 0.001 {
      self.soundtouch.set_tempo(tempo as f64);
      self.current_tempo = tempo;
    }

    // Target reservoir size: just enough to satisfy this chunk plus a small buffer
    let target_reservoir = frames_needed * 2;

    let mut frames_fed = 0;

    // Feed input only if reservoir is below target
    while self.reservoir.len() / channels < target_reservoir {
      let remaining = total_frames.saturating_sub(position + frames_fed);
      if remaining == 0 {
        break;
      }

      // Feed smaller chunks for lower latency
      let chunk_size = remaining.min(1024);
      let start_idx = (position + frames_fed) * channels;
      let end_idx = start_idx + chunk_size * channels;

      if end_idx <= pcm_data.len() {
        self
          .soundtouch
          .put_samples(&pcm_data[start_idx..end_idx], chunk_size);
        frames_fed += chunk_size;
      }

      // Process and collect output into reservoir
      self.collect_output();
    }

    // Collect any remaining output
    self.collect_output();

    // Copy from reservoir to output
    let available = self.reservoir.len() / channels;
    let to_copy = available.min(frames_needed);

    if to_copy > 0 {
      let copy_samples = to_copy * channels;
      output[..copy_samples].copy_from_slice(&self.reservoir[..copy_samples]);
      self.reservoir.drain(..copy_samples);
    }

    // Fill remaining with silence
    if to_copy < frames_needed {
      let start = to_copy * channels;
      for sample in &mut output[start..frames_needed * channels] {
        *sample = 0.0;
      }
    }

    frames_fed
  }

  /// Collect all available output from SoundTouch into reservoir
  fn collect_output(&mut self) {
    let channels = DEFAULT_CHANNELS as usize;
    let buf_frames = self.output_buffer.len() / channels;
    loop {
      let received = self
        .soundtouch
        .receive_samples(&mut self.output_buffer, buf_frames);
      if received == 0 {
        break;
      }
      self
        .reservoir
        .extend_from_slice(&self.output_buffer[..received * channels]);
    }
  }

  fn clear(&mut self) {
    self.soundtouch.clear();
    self.reservoir.clear();
  }
}

/// Deck state for a single deck
struct DeckState {
  /// PCM data (stereo interleaved f32)
  pcm_data: Option<Vec<f32>>,
  /// Current playback position in frames (updated during audio processing)
  position: usize,
  /// Whether the deck is currently playing
  playing: bool,
  /// Track BPM (if detected)
  bpm: Option<f32>,
  /// Playback rate (1.0 = normal speed)
  rate: f32,
  /// Deck gain (0.0 to 1.0)
  gain: f32,
  /// Track ID for state updates
  track_id: Option<String>,
  /// Time stretcher for pitch-preserved tempo adjustment
  time_stretcher: TimeStretcher,
  /// 3-band EQ processor
  eq_processor: EqProcessor,
  /// Loop enabled
  loop_enabled: bool,
  /// Loop start position in frames
  loop_start: usize,
  /// Loop end position in frames
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

/// Crossfade state
struct CrossfadeState {
  /// Current crossfader position (0.0 = full A, 1.0 = full B)
  position: f32,
  /// Is auto crossfade in progress
  active: bool,
  /// Auto crossfade direction
  direction: Option<CrossfadeDirection>,
  /// Remaining frames in auto crossfade
  remaining_frames: usize,
  /// Total frames for auto crossfade
  total_frames: usize,
  /// Start position for auto crossfade
  start_position: f32,
  /// Target position for auto crossfade
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
enum CrossfadeDirection {
  AtoB,
  BtoA,
}

/// Level meter state
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

/// Audio channel configuration
struct ChannelConfig {
  /// Output channel count
  output_channels: u16,
  /// Main output channels [left, right]
  main_channels: [Option<u16>; 2],
  /// Cue output channels [left, right]
  cue_channels: [Option<u16>; 2],
  /// Cue enabled for deck A
  deck_a_cue: bool,
  /// Cue enabled for deck B
  deck_b_cue: bool,
}

impl Default for ChannelConfig {
  fn default() -> Self {
    Self {
      output_channels: 2,
      main_channels: [Some(0), Some(1)],
      cue_channels: [None, None],
      deck_a_cue: false,
      deck_b_cue: false,
    }
  }
}

/// Microphone input state
struct MicrophoneState {
  /// Whether microphone is enabled
  enabled: bool,
  /// Microphone gain (0.0 to 2.0)
  gain: f32,
  /// Talkover ducking level (0.0 to 1.0, how much to reduce music)
  talkover_ducking: f32,
  /// Input buffer from microphone (ring buffer)
  input_buffer: VecDeque<f32>,
  /// Current microphone peak level
  peak: f32,
}

impl Default for MicrophoneState {
  fn default() -> Self {
    Self {
      enabled: false,
      gain: 1.0,
      talkover_ducking: 0.5, // Reduce music to 50% when talkover active
      input_buffer: VecDeque::new(),
      peak: 0.0,
    }
  }
}

/// Shared engine state protected by mutex
struct EngineState {
  deck_a: DeckState,
  deck_b: DeckState,
  crossfade: CrossfadeState,
  levels: LevelMeterState,
  channel_config: ChannelConfig,
  microphone: MicrophoneState,
  master_tempo: f32,
  running: bool,
  /// Set to true during device reconfiguration to pause audio processing
  configuring: bool,
  /// Whether microphone input is available
  mic_available: bool,
  output_queue: VecDeque<f32>,
  /// Pending state update reason (None = periodic, Some = specific event)
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
      output_queue: VecDeque::new(),
      update_reason: None,
    }
  }
}

/// EQ cut state for a deck
#[napi(object)]
#[derive(Clone, Copy, Default)]
pub struct EqCutStateJs {
  pub low: bool,
  pub mid: bool,
  pub high: bool,
}

/// Loop state for a deck
#[napi(object)]
#[derive(Clone, Copy, Default)]
pub struct LoopStateJs {
  /// Whether loop is enabled
  pub enabled: bool,
  /// Loop start position (0.0-1.0)
  pub start: f64,
  /// Loop end position (0.0-1.0)
  pub end: f64,
}

/// State update sent to JavaScript
#[napi(object)]
pub struct AudioEngineStateUpdate {
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
  /// EQ cut state for deck A
  pub deck_a_eq_cut: EqCutStateJs,
  /// EQ cut state for deck B
  pub deck_b_eq_cut: EqCutStateJs,
  /// Loop state for deck A
  pub deck_a_loop: LoopStateJs,
  /// Loop state for deck B
  pub deck_b_loop: LoopStateJs,
  /// Microphone available (input stream created successfully)
  pub mic_available: bool,
  /// Microphone enabled
  pub mic_enabled: bool,
  /// Microphone peak level
  pub mic_peak: f64,
  /// Reason for this state update: "periodic", "seek", "play", "stop", "load", etc.
  pub update_reason: String,
}

/// Device configuration for configureDevice()
#[napi(object)]
pub struct DeviceConfig {
  /// Device ID (device name, stable across restarts)
  pub device_id: Option<String>,
  /// Main output channels [left, right], -1 for disabled
  pub main_channels: Option<Vec<i32>>,
  /// Cue output channels [left, right], -1 for disabled
  pub cue_channels: Option<Vec<i32>>,
}

#[napi]
pub struct AudioEngine {
  state: Arc<Mutex<EngineState>>,
  stream: Arc<Mutex<Option<cpal::Stream>>>,
  input_stream: Arc<Mutex<Option<cpal::Stream>>>,
  _process_thread: Option<JoinHandle<()>>,
  sample_rate: u32,
}

#[napi]
impl AudioEngine {
  /// Create a new AudioEngine instance
  #[napi(constructor)]
  pub fn new(
    _device_id: Option<String>,
    _channels: Option<u16>,
    sample_rate: Option<u32>,
    #[napi(ts_arg_type = "(state: AudioEngineStateUpdate) => void")] state_callback: Function<
      AudioEngineStateUpdate,
      (),
    >,
  ) -> Result<Self> {
    let sample_rate = sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE);
    let output_channels = DEFAULT_CHANNELS;

    let state = Arc::new(Mutex::new(EngineState::new(sample_rate)));
    state.lock().channel_config.output_channels = output_channels;

    let state_for_process = Arc::clone(&state);

    // Create threadsafe function for state updates
    let tsfn = state_callback
      .build_threadsafe_function()
      .callee_handled::<false>()
      .build()?;

    // Processing thread - generates audio and sends state updates
    let sample_rate_for_process = sample_rate;
    let process_thread = thread::spawn(move || {
      // Set high thread priority for real-time audio processing
      match set_current_thread_priority(ThreadPriority::Max) {
        Ok(_) => eprintln!("[AudioEngine] Process thread priority set to Max"),
        Err(e) => eprintln!("[AudioEngine] Warning: Could not set thread priority: {e:?}"),
      }

      let target_queue_samples = (sample_rate_for_process as usize / 10) * output_channels as usize;
      let interval = Duration::from_micros(
        ((FRAMES_PER_CHUNK as f64 / sample_rate_for_process as f64) * 1_000_000.0 * 0.8) as u64,
      );
      let mut last_state_emit = Instant::now();
      let state_emit_interval = Duration::from_millis(33); // 30 FPS

      loop {
        let should_exit = {
          let state = state_for_process.lock();
          !state.running
        };

        if should_exit {
          break;
        }

        // Check queue size and get current output_channels
        let (queue_size, current_output_channels) = {
          let state = state_for_process.lock();
          (
            state.output_queue.len(),
            state.channel_config.output_channels,
          )
        };

        if queue_size < target_queue_samples * 2 {
          // Process audio chunk
          let chunk = {
            let mut state = state_for_process.lock();
            let (chunk, _) =
              process_audio_chunk(&mut state, sample_rate_for_process, current_output_channels);
            chunk
          };

          // Add to queue
          {
            let mut state = state_for_process.lock();
            state.output_queue.extend(chunk);
          }
        }

        // Emit state update at 30 FPS (always, regardless of queue size)
        if last_state_emit.elapsed() >= state_emit_interval {
          let state_update = {
            let state = state_for_process.lock();
            create_state_update(&state, sample_rate_for_process)
          };
          tsfn.call(state_update, ThreadsafeFunctionCallMode::NonBlocking);
          last_state_emit = Instant::now();
        }

        thread::sleep(interval);
      }
    });

    Ok(Self {
      state,
      stream: Arc::new(Mutex::new(None)),
      input_stream: Arc::new(Mutex::new(None)),
      _process_thread: Some(process_thread),
      sample_rate,
    })
  }

  /// Configure audio device and start output stream
  /// Can be called multiple times to switch devices without losing engine state
  #[napi]
  pub fn configure_device(&mut self, config: DeviceConfig) -> Result<()> {
    // Get device once and reuse for both output and input
    let device = get_device(config.device_id.as_deref())?;
    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

    // Get device's max output channels (use all available)
    let output_channels = device
      .default_output_config()
      .map_err(|e| Error::from_reason(format!("Device '{}' error: {}", device_name, e)))?
      .channels();

    // Stop old stream explicitly before dropping
    {
      let mut stream_guard = self.stream.lock();
      if let Some(ref stream) = *stream_guard {
        // Explicitly pause the stream before dropping
        if let Err(e) = stream.pause() {
          eprintln!("[AudioEngine] Warning: Failed to pause old stream: {e}");
        }
      }
      // Drop the old stream
      *stream_guard = None;
    }

    // Update channel config in state and clear queue
    {
      let mut state = self.state.lock();
      state.channel_config.output_channels = output_channels;

      // Log input config
      eprintln!(
        "[AudioEngine] configureDevice input: main={:?}, cue={:?}",
        config.main_channels, config.cue_channels
      );

      // Helper to clamp channel to valid range, or None if out of bounds
      let clamp_channel = |c: i32| -> Option<u16> {
        if c >= 0 && (c as u16) < output_channels {
          Some(c as u16)
        } else {
          None
        }
      };

      // Apply main/cue channel mapping (clamp to device's channel count)
      if let Some(ref main) = config.main_channels {
        state.channel_config.main_channels = [
          main.first().copied().and_then(&clamp_channel),
          main.get(1).copied().and_then(&clamp_channel),
        ];
      } else {
        // No config provided: default to channels 0 and 1
        state.channel_config.main_channels =
          [Some(0), Some(1.min(output_channels.saturating_sub(1)))];
      }

      if let Some(ref cue) = config.cue_channels {
        state.channel_config.cue_channels = [
          cue.first().copied().and_then(&clamp_channel),
          cue.get(1).copied().and_then(&clamp_channel),
        ];
      }

      // Clear output queue (old data has wrong channel count)
      state.output_queue.clear();
    }

    // Build and start new output stream
    let new_stream = build_output_stream(&device, output_channels, Arc::clone(&self.state))?;

    // Set new output stream
    {
      let mut stream_guard = self.stream.lock();
      *stream_guard = Some(new_stream);
    }

    // Try to build input stream for microphone (using same device)
    let new_input_stream = build_input_stream(&device, Arc::clone(&self.state));

    // Check if mic is available
    let has_mic = new_input_stream.is_some();

    // Set new input stream
    {
      let mut input_guard = self.input_stream.lock();
      *input_guard = new_input_stream;
    }

    // Resume process thread and log detailed config
    {
      let mut state = self.state.lock();
      state.configuring = false;
      state.mic_available = has_mic;
      eprintln!(
        "[AudioEngine] Device configured: channels={}, sample_rate={}, main={:?}, cue={:?}, mic={}",
        output_channels,
        self.sample_rate,
        state.channel_config.main_channels,
        state.channel_config.cue_channels,
        if has_mic { "available" } else { "N/A" }
      );
    }

    Ok(())
  }

  /// Load PCM data onto a deck
  #[napi]
  pub fn load_track(
    &self,
    deck: u32,
    pcm_data: Float32Array,
    bpm: Option<f64>,
    track_id: Option<String>,
  ) -> Result<()> {
    let mut state = self.state.lock();
    let master_tempo = state.master_tempo;
    let deck_state = if deck == 1 {
      &mut state.deck_a
    } else {
      &mut state.deck_b
    };

    deck_state.pcm_data = Some(pcm_data.to_vec());
    deck_state.position = 0;
    deck_state.playing = false;
    deck_state.bpm = bpm.map(|b| b as f32);
    deck_state.rate = calculate_playback_rate(bpm.map(|b| b as f32), master_tempo);
    deck_state.track_id = track_id;
    deck_state.time_stretcher.clear();

    state.update_reason = Some("load".to_string());

    Ok(())
  }

  /// Start playback on a deck
  #[napi]
  pub fn play(&self, deck: u32) -> Result<()> {
    let mut state = self.state.lock();
    if deck == 1 {
      if state.deck_a.pcm_data.is_some() {
        state.deck_a.playing = true;
      }
    } else if state.deck_b.pcm_data.is_some() {
      state.deck_b.playing = true;
    }
    state.update_reason = Some("play".to_string());
    Ok(())
  }

  /// Stop playback on a deck
  #[napi]
  pub fn stop(&self, deck: u32) -> Result<()> {
    let mut state = self.state.lock();
    if deck == 1 {
      state.deck_a.playing = false;
    } else {
      state.deck_b.playing = false;
    }
    // Reset crossfade state
    state.crossfade.active = false;
    state.crossfade.direction = None;
    state.crossfade.remaining_frames = 0;
    state.update_reason = Some("stop".to_string());
    Ok(())
  }

  /// Seek within a deck (position: 0.0 to 1.0)
  #[napi]
  pub fn seek(&self, deck: u32, position: f64) -> Result<()> {
    let position = position.clamp(0.0, 1.0);
    let mut state = self.state.lock();

    let deck_state = if deck == 1 {
      &mut state.deck_a
    } else {
      &mut state.deck_b
    };

    if let Some(ref pcm) = deck_state.pcm_data {
      let total_frames = pcm.len() / DEFAULT_CHANNELS as usize;
      deck_state.position = (total_frames as f64 * position) as usize;
      deck_state.time_stretcher.clear();
    }

    // Mark that a seek operation occurred
    state.update_reason = Some("seek".to_string());

    Ok(())
  }

  /// Set crossfader position (0.0 = full A, 1.0 = full B)
  #[napi]
  pub fn set_crossfader_position(&self, position: f64) -> Result<()> {
    let mut state = self.state.lock();
    state.crossfade.position = position.clamp(0.0, 1.0) as f32;
    Ok(())
  }

  /// Start auto crossfade
  #[napi]
  pub fn start_crossfade(&self, target_position: Option<f64>, duration: f64) -> Result<()> {
    let mut state = self.state.lock();
    let current = state.crossfade.position;

    let target = target_position
      .map(|p| p.clamp(0.0, 1.0) as f32)
      .unwrap_or(if state.deck_a.playing { 1.0 } else { 0.0 });

    let direction = if target > current {
      CrossfadeDirection::AtoB
    } else {
      CrossfadeDirection::BtoA
    };

    let total_frames = (duration * self.sample_rate as f64) as usize;

    state.crossfade.active = true;
    state.crossfade.direction = Some(direction);
    state.crossfade.remaining_frames = total_frames;
    state.crossfade.total_frames = total_frames;
    state.crossfade.start_position = current;
    state.crossfade.target_position = target;

    Ok(())
  }

  /// Set master tempo (BPM)
  #[napi]
  pub fn set_master_tempo(&self, bpm: f64) -> Result<()> {
    if bpm <= 0.0 || bpm > 300.0 {
      return Ok(());
    }

    let mut state = self.state.lock();
    state.master_tempo = bpm as f32;

    // Update playback rates (SoundTouch handles tempo changes smoothly without clearing)
    state.deck_a.rate = calculate_playback_rate(state.deck_a.bpm, state.master_tempo);
    state.deck_b.rate = calculate_playback_rate(state.deck_b.bpm, state.master_tempo);

    Ok(())
  }

  /// Set deck gain (0.0 to 1.0)
  #[napi]
  pub fn set_deck_gain(&self, deck: u32, gain: f64) -> Result<()> {
    let gain = gain.clamp(0.0, 1.0) as f32;
    // Apply logarithmic curve for natural volume control
    let db_gain = if gain == 0.0 { 0.0 } else { gain * gain };

    let mut state = self.state.lock();
    if deck == 1 {
      state.deck_a.gain = db_gain;
    } else {
      state.deck_b.gain = db_gain;
    }
    Ok(())
  }

  /// Set EQ cut (kill switch) for a specific band on a deck
  /// band: "low", "mid", "high"
  #[napi]
  pub fn set_eq_cut(&self, deck: u32, band: String, enabled: bool) -> Result<()> {
    let eq_band = match band.as_str() {
      "low" => EqBand::Low,
      "mid" => EqBand::Mid,
      "high" => EqBand::High,
      _ => return Err(Error::from_reason(format!("Invalid EQ band: {}", band))),
    };

    let mut state = self.state.lock();
    if deck == 1 {
      state.deck_a.eq_processor.set_cut(eq_band, enabled);
    } else {
      state.deck_b.eq_processor.set_cut(eq_band, enabled);
    }
    Ok(())
  }

  /// Get EQ cut state for a deck
  #[napi]
  pub fn get_eq_cut_state(&self, deck: u32) -> Result<EqCutStateJs> {
    let state = self.state.lock();
    let eq_state = if deck == 1 {
      state.deck_a.eq_processor.get_cut_state()
    } else {
      state.deck_b.eq_processor.get_cut_state()
    };
    Ok(EqCutStateJs {
      low: eq_state.low,
      mid: eq_state.mid,
      high: eq_state.high,
    })
  }

  /// Set cue enabled for a deck
  #[napi]
  pub fn set_deck_cue_enabled(&self, deck: u32, enabled: bool) -> Result<()> {
    let mut state = self.state.lock();
    if deck == 1 {
      state.channel_config.deck_a_cue = enabled;
    } else {
      state.channel_config.deck_b_cue = enabled;
    }
    Ok(())
  }

  /// Set channel configuration for main and cue outputs
  /// channel values: -1 means disabled, 0+ means the output channel index
  #[napi]
  pub fn set_channel_config(
    &self,
    main_left: i32,
    main_right: i32,
    cue_left: i32,
    cue_right: i32,
  ) -> Result<()> {
    let mut state = self.state.lock();
    state.channel_config.main_channels = [
      if main_left >= 0 {
        Some(main_left as u16)
      } else {
        None
      },
      if main_right >= 0 {
        Some(main_right as u16)
      } else {
        None
      },
    ];
    state.channel_config.cue_channels = [
      if cue_left >= 0 {
        Some(cue_left as u16)
      } else {
        None
      },
      if cue_right >= 0 {
        Some(cue_right as u16)
      } else {
        None
      },
    ];
    // Calculate required output channels
    let max_channel = [main_left, main_right, cue_left, cue_right]
      .iter()
      .filter(|&&c| c >= 0)
      .max()
      .copied()
      .unwrap_or(1) as u16;
    state.channel_config.output_channels = max_channel + 1;
    Ok(())
  }

  /// Get current state
  #[napi]
  pub fn get_state(&self) -> Result<AudioEngineStateUpdate> {
    let state = self.state.lock();
    Ok(create_state_update(&state, self.sample_rate))
  }

  /// Enable or disable microphone input
  #[napi]
  pub fn set_mic_enabled(&self, enabled: bool) -> Result<()> {
    let mut state = self.state.lock();
    state.microphone.enabled = enabled;
    if !enabled {
      state.microphone.input_buffer.clear();
      state.microphone.peak = 0.0;
    }
    eprintln!(
      "[AudioEngine] Microphone {}",
      if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
  }

  /// Set microphone gain
  #[napi]
  pub fn set_mic_gain(&self, gain: f64) -> Result<()> {
    let mut state = self.state.lock();
    state.microphone.gain = (gain as f32).clamp(0.0, 2.0);
    Ok(())
  }

  /// Set talkover ducking level (0.0 to 1.0 - how much to reduce music)
  #[napi]
  pub fn set_talkover_ducking(&self, ducking: f64) -> Result<()> {
    let mut state = self.state.lock();
    state.microphone.talkover_ducking = (ducking as f32).clamp(0.0, 1.0);
    Ok(())
  }

  /// Set loop region for a deck (positions in 0.0-1.0 range)
  #[napi]
  pub fn set_loop(&self, deck: u32, start: f64, end: f64, enabled: bool) -> Result<()> {
    let mut state = self.state.lock();
    let deck_state = if deck == 1 {
      &mut state.deck_a
    } else {
      &mut state.deck_b
    };

    if let Some(ref pcm) = deck_state.pcm_data {
      let total_frames = pcm.len() / DEFAULT_CHANNELS as usize;
      deck_state.loop_start = (total_frames as f64 * start.clamp(0.0, 1.0)) as usize;
      deck_state.loop_end = (total_frames as f64 * end.clamp(0.0, 1.0)) as usize;
      deck_state.loop_enabled = enabled && deck_state.loop_end > deck_state.loop_start;
    }

    Ok(())
  }

  /// Set beat loop for a deck using beat grid positions
  /// start_seconds and end_seconds are calculated from beat grid on TypeScript side
  #[napi]
  pub fn set_beat_loop(&self, deck: u32, start_seconds: f64, end_seconds: f64) -> Result<()> {
    let mut state = self.state.lock();
    let deck_state = if deck == 1 {
      &mut state.deck_a
    } else {
      &mut state.deck_b
    };

    if let Some(ref pcm) = deck_state.pcm_data {
      let total_frames = pcm.len() / DEFAULT_CHANNELS as usize;
      let sample_rate = DEFAULT_SAMPLE_RATE as f64;

      let loop_start = (start_seconds * sample_rate) as usize;
      let loop_end = ((end_seconds * sample_rate) as usize).min(total_frames);

      if loop_end > loop_start {
        deck_state.loop_start = loop_start;
        deck_state.loop_end = loop_end;
        deck_state.loop_enabled = true;

        // Jump to loop start if currently past loop end or before loop start
        if deck_state.position >= loop_end || deck_state.position < loop_start {
          deck_state.position = loop_start;
          deck_state.time_stretcher.clear();
        }
      }
    }

    Ok(())
  }

  /// Clear loop for a deck
  #[napi]
  pub fn clear_loop(&self, deck: u32) -> Result<()> {
    let mut state = self.state.lock();
    let deck_state = if deck == 1 {
      &mut state.deck_a
    } else {
      &mut state.deck_b
    };

    deck_state.loop_enabled = false;
    deck_state.loop_start = 0;
    deck_state.loop_end = 0;

    Ok(())
  }

  /// Clean up and stop the engine
  #[napi]
  pub fn close(&self) -> Result<()> {
    // Stop the streams first
    {
      let mut stream_guard = self.stream.lock();
      *stream_guard = None;
    }
    {
      let mut input_guard = self.input_stream.lock();
      *input_guard = None;
    }

    let mut state = self.state.lock();
    state.running = false;
    state.deck_a.playing = false;
    state.deck_b.playing = false;
    state.output_queue.clear();
    Ok(())
  }
}

/// Get device's max output channels
/// Find audio device by name, or return default output device
fn get_device(device_id: Option<&str>) -> Result<cpal::Device> {
  let host = cpal::default_host();

  if let Some(name) = device_id {
    // Find device by name (stable across restarts, unlike index)
    for dev in host.devices().map_err(map_err)? {
      if let Ok(dev_name) = dev.name() {
        if dev_name == name {
          return Ok(dev);
        }
      }
    }
    // Fallback to default if device not found
    eprintln!("[AudioEngine] Device '{}' not found, using default", name);
  }

  host
    .default_output_device()
    .ok_or_else(|| Error::from_reason("No default output device available"))
}

/// Build an audio output stream for the given device
fn build_output_stream(
  device: &cpal::Device,
  output_channels: u16,
  state: Arc<Mutex<EngineState>>,
) -> Result<cpal::Stream> {
  let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
  eprintln!("[AudioEngine] Using device: {}", device_name);

  let config = device.default_output_config().map_err(|e| {
    Error::from_reason(format!(
      "Device '{}' does not support output: {}",
      device_name, e
    ))
  })?;

  if config.sample_format() != SampleFormat::F32 {
    return Err(Error::from_reason("Device does not support f32 output"));
  }

  let mut final_config = config.config();
  final_config.channels = output_channels;

  let state_for_audio = Arc::clone(&state);

  let stream = device
    .build_output_stream(
      &final_config,
      move |data: &mut [f32], _| {
        let mut state = state_for_audio.lock();
        for sample in data.iter_mut() {
          *sample = state.output_queue.pop_front().unwrap_or(0.0);
        }
      },
      move |err| eprintln!("[AudioEngine] Output stream error: {err}"),
      None,
    )
    .map_err(|e| Error::from_reason(format!("Failed to build audio stream: {e}")))?;

  stream
    .play()
    .map_err(|e| Error::from_reason(format!("Failed to start audio stream: {e}")))?;

  Ok(stream)
}

/// Build an audio input stream for microphone using the same device as output
fn build_input_stream(
  device: &cpal::Device,
  state: Arc<Mutex<EngineState>>,
) -> Option<cpal::Stream> {
  let input_config = match device.default_input_config() {
    Ok(config) => config,
    Err(_) => {
      // Device doesn't support input (e.g., output-only device)
      return None;
    }
  };

  if input_config.sample_format() != SampleFormat::F32 {
    eprintln!("[AudioEngine] Input device does not support f32 format");
    return None;
  }

  let input_sample_rate = input_config.sample_rate().0;
  let input_channels = input_config.channels();

  let state_for_input = Arc::clone(&state);

  match device.build_input_stream(
    &input_config.into(),
    move |data: &[f32], _| {
      let mut state = state_for_input.lock();

      // Always buffer and track peak level (regardless of enabled state)
      // Use first channel only (mono mic) and duplicate to stereo
      let ch = input_channels as usize;
      let frames = data.len() / ch;

      for frame in 0..frames {
        let sample = data[frame * ch]; // First channel only
        state.microphone.input_buffer.push_back(sample);
        state.microphone.input_buffer.push_back(sample); // Duplicate to stereo
      }

      // Limit buffer size (keep ~100ms of audio at stereo)
      let max_samples = (input_sample_rate as usize / 10) * 2;
      while state.microphone.input_buffer.len() > max_samples {
        state.microphone.input_buffer.pop_front();
      }

      // Update peak level (first channel only)
      let mut peak = 0.0f32;
      for frame in 0..frames {
        peak = peak.max(data[frame * ch].abs());
      }
      state.microphone.peak = state.microphone.peak * 0.9 + peak * 0.1;
    },
    move |err| eprintln!("[AudioEngine] Input stream error: {err}"),
    None,
  ) {
    Ok(stream) => {
      if stream.play().is_ok() {
        eprintln!(
          "[AudioEngine] Microphone input available ({} channels)",
          input_channels
        );
        Some(stream)
      } else {
        None
      }
    }
    Err(e) => {
      eprintln!("[AudioEngine] Could not create input stream: {e}");
      None
    }
  }
}

/// Calculate playback rate based on track BPM and master tempo
fn calculate_playback_rate(track_bpm: Option<f32>, master_tempo: f32) -> f32 {
  match track_bpm {
    Some(bpm) if bpm > 0.0 => (master_tempo / bpm).clamp(0.5, 2.0),
    _ => 1.0,
  }
}

/// Process a single audio chunk
fn process_audio_chunk(
  state: &mut EngineState,
  sample_rate: u32,
  output_channels: u16,
) -> (Vec<f32>, AudioEngineStateUpdate) {
  let frames = FRAMES_PER_CHUNK;
  let channels = DEFAULT_CHANNELS as usize;

  // Pre-allocate buffers
  let mut buffer_a = vec![0.0f32; frames * channels];
  let mut buffer_b = vec![0.0f32; frames * channels];
  let mut mix_buffer = vec![0.0f32; frames * channels];

  // Process deck A with time stretching
  if state.deck_a.playing {
    if let Some(ref pcm) = state.deck_a.pcm_data {
      let total_frames = pcm.len() / channels;
      let rate = state.deck_a.rate;

      // Use time stretcher for tempo adjustment with pitch preservation
      let frames_consumed = state.deck_a.time_stretcher.process(
        pcm,
        state.deck_a.position,
        rate,
        frames,
        &mut buffer_a,
      );

      // Apply EQ processing
      state.deck_a.eq_processor.process(&mut buffer_a, frames);

      state.deck_a.position += frames_consumed;

      // Check for loop or track end
      if state.deck_a.loop_enabled && state.deck_a.position >= state.deck_a.loop_end {
        // Loop back to start
        state.deck_a.position = state.deck_a.loop_start;
        state.deck_a.time_stretcher.clear();
      } else if state.deck_a.position >= total_frames {
        state.deck_a.playing = false;
        state.deck_a.position = 0;
        state.deck_a.time_stretcher.clear();
      }
    }
  }

  // Process deck B with time stretching
  if state.deck_b.playing {
    if let Some(ref pcm) = state.deck_b.pcm_data {
      let total_frames = pcm.len() / channels;
      let rate = state.deck_b.rate;

      // Use time stretcher for tempo adjustment with pitch preservation
      let frames_consumed = state.deck_b.time_stretcher.process(
        pcm,
        state.deck_b.position,
        rate,
        frames,
        &mut buffer_b,
      );

      // Apply EQ processing
      state.deck_b.eq_processor.process(&mut buffer_b, frames);

      state.deck_b.position += frames_consumed;

      // Check for loop or track end
      if state.deck_b.loop_enabled && state.deck_b.position >= state.deck_b.loop_end {
        // Loop back to start
        state.deck_b.position = state.deck_b.loop_start;
        state.deck_b.time_stretcher.clear();
      } else if state.deck_b.position >= total_frames {
        state.deck_b.playing = false;
        state.deck_b.position = 0;
        state.deck_b.time_stretcher.clear();
      }
    }
  }

  // Handle auto crossfade
  if state.crossfade.active && state.crossfade.remaining_frames > 0 {
    state.crossfade.remaining_frames = state.crossfade.remaining_frames.saturating_sub(frames);

    if state.crossfade.remaining_frames == 0 {
      // Crossfade complete
      state.crossfade.position = state.crossfade.target_position;

      if let Some(dir) = state.crossfade.direction {
        match dir {
          CrossfadeDirection::AtoB => {
            state.deck_a.playing = false;
            state.deck_b.playing = true;
          }
          CrossfadeDirection::BtoA => {
            state.deck_b.playing = false;
            state.deck_a.playing = true;
          }
        }
      }

      state.crossfade.active = false;
      state.crossfade.direction = None;
    } else {
      // Update crossfader position during crossfade
      let progress =
        1.0 - (state.crossfade.remaining_frames as f32 / state.crossfade.total_frames as f32);
      state.crossfade.position = state.crossfade.start_position
        + (state.crossfade.target_position - state.crossfade.start_position) * progress;

      // Start target deck if not playing
      if let Some(dir) = state.crossfade.direction {
        match dir {
          CrossfadeDirection::AtoB if !state.deck_b.playing => {
            state.deck_b.playing = true;
          }
          CrossfadeDirection::BtoA if !state.deck_a.playing => {
            state.deck_a.playing = true;
          }
          _ => {}
        }
      }
    }
  }

  // Apply crossfader with Pioneer-style constant power curve
  let position = state.crossfade.position;
  let gain_a = if state.deck_a.playing {
    (position * PI / 2.0).cos()
  } else {
    0.0
  };
  let gain_b = if state.deck_b.playing {
    (position * PI / 2.0).sin()
  } else {
    0.0
  };

  let deck_a_gain = gain_a * state.deck_a.gain;
  let deck_b_gain = gain_b * state.deck_b.gain;

  // Calculate peak levels (post deck-gain, pre-crossfade)
  state.levels.deck_a_peak = calculate_peak(&buffer_a, frames) * state.deck_a.gain;
  state.levels.deck_b_peak = calculate_peak(&buffer_b, frames) * state.deck_b.gain;

  // Update peak hold
  update_peak_hold(&mut state.levels);

  // Mix decks
  for i in 0..(frames * channels) {
    mix_buffer[i] = buffer_a[i] * deck_a_gain + buffer_b[i] * deck_b_gain;
  }

  // Apply microphone input and talkover
  apply_mic_talkover(state, &mut mix_buffer, frames);

  // Map to output channels
  // Always use map_channels if cue is enabled or channel mapping is non-default
  let needs_channel_mapping = output_channels as usize != channels
    || state.channel_config.deck_a_cue
    || state.channel_config.deck_b_cue
    || state.channel_config.cue_channels[0].is_some()
    || state.channel_config.cue_channels[1].is_some();

  let output = if needs_channel_mapping {
    map_channels(
      &mix_buffer,
      frames,
      output_channels,
      &state.channel_config,
      &buffer_a,
      &buffer_b,
    )
  } else {
    // Clip output
    mix_buffer.iter().map(|s| s.clamp(-1.0, 1.0)).collect()
  };

  let state_update = create_state_update(state, sample_rate);

  // Reset pending reason after creating state update
  state.update_reason = None;

  (output, state_update)
}

/// Calculate peak level from buffer
fn calculate_peak(buffer: &[f32], frames: usize) -> f32 {
  let channels = DEFAULT_CHANNELS as usize;
  let available = frames.min(buffer.len() / channels);
  let mut peak = 0.0f32;

  for i in 0..available {
    for ch in 0..channels {
      peak = peak.max(buffer[i * channels + ch].abs());
    }
  }

  peak
}

/// Update peak hold values
fn update_peak_hold(levels: &mut LevelMeterState) {
  const HOLD_DURATION: Duration = Duration::from_millis(1500);
  const DECAY_RATE: f32 = 6.0; // dB per second

  let now = Instant::now();

  // Deck A
  if levels.deck_a_peak > levels.deck_a_peak_hold {
    levels.deck_a_peak_hold = levels.deck_a_peak;
    levels.deck_a_peak_hold_time = now;
  } else if now.duration_since(levels.deck_a_peak_hold_time) > HOLD_DURATION {
    let decay_time =
      (now.duration_since(levels.deck_a_peak_hold_time) - HOLD_DURATION).as_secs_f32();
    let decay_db = DECAY_RATE * decay_time;
    let current_db = if levels.deck_a_peak_hold > 0.0 {
      20.0 * levels.deck_a_peak_hold.log10()
    } else {
      f32::NEG_INFINITY
    };
    let new_db = current_db - decay_db;
    levels.deck_a_peak_hold = if new_db == f32::NEG_INFINITY {
      0.0
    } else {
      10.0f32.powf(new_db / 20.0).max(levels.deck_a_peak)
    };
  }

  // Deck B
  if levels.deck_b_peak > levels.deck_b_peak_hold {
    levels.deck_b_peak_hold = levels.deck_b_peak;
    levels.deck_b_peak_hold_time = now;
  } else if now.duration_since(levels.deck_b_peak_hold_time) > HOLD_DURATION {
    let decay_time =
      (now.duration_since(levels.deck_b_peak_hold_time) - HOLD_DURATION).as_secs_f32();
    let decay_db = DECAY_RATE * decay_time;
    let current_db = if levels.deck_b_peak_hold > 0.0 {
      20.0 * levels.deck_b_peak_hold.log10()
    } else {
      f32::NEG_INFINITY
    };
    let new_db = current_db - decay_db;
    levels.deck_b_peak_hold = if new_db == f32::NEG_INFINITY {
      0.0
    } else {
      10.0f32.powf(new_db / 20.0).max(levels.deck_b_peak)
    };
  }
}

/// Apply microphone input and talkover to mixed audio
fn apply_mic_talkover(state: &mut EngineState, mix_buffer: &mut [f32], frames: usize) {
  let channels = DEFAULT_CHANNELS as usize;
  let mic = &mut state.microphone;

  // Check if we have enough mic samples
  let available_samples = mic.input_buffer.len();
  let needed_samples = frames * channels;

  if available_samples < needed_samples {
    // Not enough mic data, skip but don't reset peak (preserve last value briefly)
    return;
  }

  // Calculate music attenuation and mic gain only when enabled
  let (music_attenuation, mic_gain) = if mic.enabled {
    (1.0 - mic.talkover_ducking, mic.gain)
  } else {
    (1.0, 0.0) // No ducking, no mic output when disabled
  };

  let mut peak = 0.0f32;

  for i in 0..frames {
    let base = i * channels;

    // Read mic sample (always consume from buffer to keep it flowing)
    let mic_left = mic.input_buffer.pop_front().unwrap_or(0.0);
    let mic_right = if channels > 1 {
      mic.input_buffer.pop_front().unwrap_or(mic_left)
    } else {
      mic_left
    };

    // Track peak level (always, regardless of enabled state)
    peak = peak.max(mic_left.abs()).max(mic_right.abs());

    // Apply talkover: attenuate music and add mic (only when enabled)
    mix_buffer[base] = mix_buffer[base] * music_attenuation + mic_left * mic_gain;
    if channels > 1 {
      mix_buffer[base + 1] = mix_buffer[base + 1] * music_attenuation + mic_right * mic_gain;
    }
  }

  // Update peak level
  mic.peak = peak;
}

/// Map stereo mix to output channels with main/cue routing
fn map_channels(
  mix: &[f32],
  frames: usize,
  output_channels: u16,
  config: &ChannelConfig,
  buffer_a: &[f32],
  buffer_b: &[f32],
) -> Vec<f32> {
  let channels = DEFAULT_CHANNELS as usize;
  let out_ch = output_channels as usize;
  let mut output = vec![0.0f32; frames * out_ch];

  let [main_l, main_r] = config.main_channels;
  let [cue_l, cue_r] = config.cue_channels;

  for frame in 0..frames {
    let mix_base = frame * channels;
    let out_base = frame * out_ch;

    let main_left = mix[mix_base];
    let main_right = mix.get(mix_base + 1).copied().unwrap_or(main_left);
    let mono_main = (main_left + main_right) * 0.5;

    // Main outputs
    if let (Some(l), Some(r)) = (main_l, main_r) {
      output[out_base + l as usize] = main_left;
      output[out_base + r as usize] = main_right;
    } else if let Some(l) = main_l {
      output[out_base + l as usize] = mono_main;
    } else if let Some(r) = main_r {
      output[out_base + r as usize] = mono_main;
    }

    // Cue outputs
    let cue_enabled = config.deck_a_cue || config.deck_b_cue;
    if cue_enabled && (cue_l.is_some() || cue_r.is_some()) {
      let mut cue_left = 0.0;
      let mut cue_right = 0.0;
      let mut cue_sources = 0;

      if config.deck_a_cue {
        cue_left += buffer_a[mix_base];
        cue_right += buffer_a
          .get(mix_base + 1)
          .copied()
          .unwrap_or(buffer_a[mix_base]);
        cue_sources += 1;
      }

      if config.deck_b_cue {
        cue_left += buffer_b[mix_base];
        cue_right += buffer_b
          .get(mix_base + 1)
          .copied()
          .unwrap_or(buffer_b[mix_base]);
        cue_sources += 1;
      }

      if cue_sources > 0 {
        let norm = 1.0 / cue_sources as f32;
        cue_left = (cue_left * norm).clamp(-1.0, 1.0);
        cue_right = (cue_right * norm).clamp(-1.0, 1.0);
        let mono_cue = (cue_left + cue_right) * 0.5;

        if let (Some(l), Some(r)) = (cue_l, cue_r) {
          output[out_base + l as usize] = cue_left;
          output[out_base + r as usize] = cue_right;
        } else if let Some(l) = cue_l {
          output[out_base + l as usize] = mono_cue;
        } else if let Some(r) = cue_r {
          output[out_base + r as usize] = mono_cue;
        }
      }
    }
  }

  // Clip output
  output.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
  output
}

/// Create state update for JavaScript
fn create_state_update(state: &EngineState, sample_rate: u32) -> AudioEngineStateUpdate {
  // Calculate position for deck A
  let deck_a_position = state
    .deck_a
    .pcm_data
    .as_ref()
    .map(|_| state.deck_a.position as f64 / sample_rate as f64);

  // Calculate position for deck B
  let deck_b_position = state
    .deck_b
    .pcm_data
    .as_ref()
    .map(|_| state.deck_b.position as f64 / sample_rate as f64);

  // Use update_reason if set, otherwise "periodic"
  let update_reason = state
    .update_reason
    .clone()
    .unwrap_or_else(|| "periodic".to_string());

  // Get EQ cut states
  let deck_a_eq = state.deck_a.eq_processor.get_cut_state();
  let deck_b_eq = state.deck_b.eq_processor.get_cut_state();

  // Calculate loop positions as normalized values (0-1)
  let channels = DEFAULT_CHANNELS as usize;
  let deck_a_loop = if let Some(ref pcm) = state.deck_a.pcm_data {
    let total_frames = pcm.len() / channels;
    LoopStateJs {
      enabled: state.deck_a.loop_enabled,
      start: state.deck_a.loop_start as f64 / total_frames as f64,
      end: state.deck_a.loop_end as f64 / total_frames as f64,
    }
  } else {
    LoopStateJs::default()
  };

  let deck_b_loop = if let Some(ref pcm) = state.deck_b.pcm_data {
    let total_frames = pcm.len() / channels;
    LoopStateJs {
      enabled: state.deck_b.loop_enabled,
      start: state.deck_b.loop_start as f64 / total_frames as f64,
      end: state.deck_b.loop_end as f64 / total_frames as f64,
    }
  } else {
    LoopStateJs::default()
  };

  AudioEngineStateUpdate {
    deck_a_position,
    deck_b_position,
    deck_a_playing: state.deck_a.playing,
    deck_b_playing: state.deck_b.playing,
    crossfader_position: state.crossfade.position as f64,
    is_crossfading: state.crossfade.active,
    deck_a_peak: state.levels.deck_a_peak as f64,
    deck_b_peak: state.levels.deck_b_peak as f64,
    deck_a_peak_hold: state.levels.deck_a_peak_hold as f64,
    deck_b_peak_hold: state.levels.deck_b_peak_hold as f64,
    master_tempo: state.master_tempo as f64,
    deck_a_track_id: state.deck_a.track_id.clone(),
    deck_b_track_id: state.deck_b.track_id.clone(),
    deck_a_gain: state.deck_a.gain as f64,
    deck_b_gain: state.deck_b.gain as f64,
    deck_a_cue_enabled: state.channel_config.deck_a_cue,
    deck_b_cue_enabled: state.channel_config.deck_b_cue,
    deck_a_eq_cut: EqCutStateJs {
      low: deck_a_eq.low,
      mid: deck_a_eq.mid,
      high: deck_a_eq.high,
    },
    deck_b_eq_cut: EqCutStateJs {
      low: deck_b_eq.low,
      mid: deck_b_eq.mid,
      high: deck_b_eq.high,
    },
    deck_a_loop,
    deck_b_loop,
    mic_available: state.mic_available,
    mic_enabled: state.microphone.enabled,
    mic_peak: state.microphone.peak as f64,
    update_reason,
  }
}

fn map_err<E: ToString>(err: E) -> Error {
  Error::from_reason(err.to_string())
}
