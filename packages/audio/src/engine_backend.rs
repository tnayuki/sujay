//! Backend boundary for the DJ engine.
//! Mix/routing backend for the DJ engine.

use std::error::Error;
use std::f32::consts::{FRAC_1_SQRT_2, PI};

use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError, TrySendError};
use napi::{Error as NapiError, Result};
use web_audio_api::context::{
  AudioContext,
  AudioContextLatencyCategory,
  AudioContextOptions,
  BaseAudioContext,
};
use web_audio_api::media_devices::{enumerate_devices_sync, MediaDeviceInfoKind};
use web_audio_api::media_streams::MediaStreamTrack;
use web_audio_api::node::{
  AudioNode,
  BiquadFilterNode,
  BiquadFilterType,
  ChannelMergerNode,
  ChannelSplitterNode,
  GainNode,
  MediaStreamTrackAudioSourceNode,
};
use web_audio_api::AudioBuffer;

/// Kill-switch state for all three EQ bands.
#[derive(Debug, Clone, Copy, Default)]
pub struct EqCutState {
  pub low: bool,
  pub mid: bool,
  pub high: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineIoConfig {
  pub output_channels: u16,
  pub main_channels: [Option<u16>; 2],
  pub cue_channels: [Option<u16>; 2],
  pub output_device_name: Option<String>,
}

impl Default for EngineIoConfig {
  fn default() -> Self {
    Self {
      output_channels: 2,
      main_channels: [Some(0), Some(1)],
      cue_channels: [None, None],
      output_device_name: None,
    }
  }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderInput<'a> {
  pub deck_a: Option<&'a [f32]>,
  pub deck_b: Option<&'a [f32]>,
  pub mic: Option<&'a [f32]>,
  pub frames: usize,
  pub crossfader_position: f32,
  pub deck_a_playing: bool,
  pub deck_b_playing: bool,
  pub deck_a_gain: f32,
  pub deck_b_gain: f32,
  pub deck_a_cue: bool,
  pub deck_b_cue: bool,
  pub talkover_ducking: f32,
  pub mic_enabled: bool,
  pub mic_gain: f32,
  pub deck_a_eq: EqCutState,
  pub deck_b_eq: EqCutState,
}

#[derive(Debug, Clone)]
pub struct RenderOutput {
  pub interleaved: Vec<f32>,
  pub deck_a_peak: f32,
  pub deck_b_peak: f32,
  pub mic_peak: f32,
}

pub struct WebAudioBackend {
  io: EngineIoConfig,
  sample_rate: f32,
  graph: Option<WebAudioGraph>,
  crossfader: CrossfaderNode,
}

impl WebAudioBackend {
  pub fn new(sample_rate: u32) -> Self {
    Self {
      io: EngineIoConfig::default(),
      sample_rate: sample_rate as f32,
      graph: None,
      crossfader: CrossfaderNode,
    }
  }

  pub fn configure_io(&mut self, config: &EngineIoConfig) -> Result<()> {
    let io_changed = self.io != *config;
    self.io = config.clone();

    if io_changed {
      self.reset_graph();
    }
    Ok(())
  }

  pub fn render(&mut self, input: RenderInput<'_>) -> Result<RenderOutput> {
    Ok(self.render_graph(input))
  }

  fn reset_graph(&mut self) {
    if let Some(graph) = self.graph.take() {
      graph.close();
    }
  }

  fn ensure_graph(&mut self) -> Result<&mut WebAudioGraph> {
    if self.graph.is_none() {
      self.graph = Some(WebAudioGraph::new(&self.io, self.sample_rate)?);
    }
    Ok(self.graph.as_mut().expect("graph initialized"))
  }

  fn render_graph(&mut self, input: RenderInput<'_>) -> RenderOutput {
    let (cross_a, cross_b) = self
      .crossfader
      .process(input.crossfader_position, input.deck_a_playing, input.deck_b_playing);

    let deck_a_gain = cross_a * input.deck_a_gain;
    let deck_b_gain = cross_b * input.deck_b_gain;

    let software_mix = mix_and_route_with_gains(&self.io, input, deck_a_gain, deck_b_gain);

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      let graph = self.ensure_graph()?;
      graph.set_mix_levels(
        deck_a_gain,
        deck_b_gain,
        if input.mic_enabled {
          (1.0 - input.talkover_ducking).clamp(0.0, 1.0)
        } else {
          1.0
        },
        if input.mic_enabled { input.mic_gain } else { 0.0 },
      );
      graph.set_deck_eq_cuts(input.deck_a_eq, input.deck_b_eq);

      graph.deck_a.push_interleaved(input.deck_a.unwrap_or(&[]), input.frames);
      graph.deck_b.push_interleaved(input.deck_b.unwrap_or(&[]), input.frames);
      graph.mic.push_interleaved(input.mic.unwrap_or(&[]), input.frames);

      let cue_mix = build_cue_mix(
        input.deck_a.unwrap_or(&[]),
        input.deck_b.unwrap_or(&[]),
        input.frames,
        input.deck_a_cue,
        input.deck_b_cue,
      );
      graph.cue.push_interleaved(&cue_mix, input.frames);

      Ok::<(), NapiError>(())
    })) {
      Ok(Ok(())) => software_mix,
      Ok(Err(err)) => {
        eprintln!("[AudioEngine] Web Audio graph update failed: {}", err);
        self.reset_graph();
        software_mix
      }
      Err(_) => {
        eprintln!("[AudioEngine] Web Audio graph panicked, resetting graph");
        self.reset_graph();
        software_mix
      }
    }
  }
}

impl Default for WebAudioBackend {
  fn default() -> Self {
    Self::new(44_100)
  }
}

#[derive(Default)]
struct CrossfaderNode;

impl CrossfaderNode {
  fn process(&self, position: f32, deck_a_playing: bool, deck_b_playing: bool) -> (f32, f32) {
    let gain_a = if deck_a_playing {
      (position * PI / 2.0).cos()
    } else {
      0.0
    };
    let gain_b = if deck_b_playing {
      (position * PI / 2.0).sin()
    } else {
      0.0
    };
    (gain_a, gain_b)
  }
}

struct BufferFeeder {
  sender: Sender<AudioBuffer>,
  receiver: Receiver<AudioBuffer>,
  channels: usize,
  sample_rate: f32,
}

impl BufferFeeder {
  fn new(channels: usize, sample_rate: f32) -> (Self, MediaStreamTrack) {
    let (sender, receiver) = bounded(8);
    let stream = RealtimeBufferStream {
      receiver: receiver.clone(),
      channels,
      sample_rate,
    };
    let track = MediaStreamTrack::from_iter(stream);

    (
      Self {
        sender,
        receiver,
        channels,
        sample_rate,
      },
      track,
    )
  }

  fn push_interleaved(&self, interleaved: &[f32], frames: usize) {
    let buffer = interleaved_to_buffer(interleaved, frames, self.channels, self.sample_rate);
    self.push_buffer(buffer);
  }

  fn push_buffer(&self, mut buffer: AudioBuffer) {
    loop {
      match self.sender.try_send(buffer) {
        Ok(()) => break,
        Err(TrySendError::Full(returned)) => {
          let _ = self.receiver.try_recv();
          buffer = returned;
        }
        Err(TrySendError::Disconnected(_)) => break,
      }
    }
  }
}

struct RealtimeBufferStream {
  receiver: Receiver<AudioBuffer>,
  channels: usize,
  sample_rate: f32,
}

impl Iterator for RealtimeBufferStream {
  type Item = std::result::Result<AudioBuffer, Box<dyn Error + Send + Sync>>;

  fn next(&mut self) -> Option<Self::Item> {
    match self.receiver.try_recv() {
      Ok(buffer) => Some(Ok(buffer)),
      Err(TryRecvError::Empty) => Some(Ok(AudioBuffer::from(
        vec![vec![0.0; 128]; self.channels.max(1)],
        self.sample_rate,
      ))),
      Err(TryRecvError::Disconnected) => None,
    }
  }
}

/// Per-deck 3-band EQ with kill switches, wired inside the Web Audio graph.
///
/// Topology (source = MediaStreamTrackAudioSourceNode):
///   source ──→ low_lp1 → low_lp2  → low_kill  ──┐
///          ↘─→ mid_hp  → mid_lp   → mid_kill  ──→ eq_bus → deck_gain
///          ↘─→ high_hp1 → high_hp2 → high_kill ──┘
struct DeckEq {
  /// Kill-switch gain nodes (0.0 = kill, 1.0 = pass)
  low_kill: GainNode,
  mid_kill: GainNode,
  high_kill: GainNode,
  /// Sum bus connecting to downstream deck gain
  _eq_bus: GainNode,
  /// Filter nodes kept alive (graph holds processing state)
  _low_lp1: BiquadFilterNode,
  _low_lp2: BiquadFilterNode,
  _mid_hp: BiquadFilterNode,
  _mid_lp: BiquadFilterNode,
  _high_hp1: BiquadFilterNode,
  _high_hp2: BiquadFilterNode,
}

impl DeckEq {
  fn new(context: &AudioContext, source: &impl AudioNode, deck_gain: &GainNode) -> Self {
    const FREQ_LOW: f32 = 250.0;
    const FREQ_HIGH: f32 = 5000.0;

    // Low band: 2× Butterworth LPF @ 250 Hz (−24 dB/oct, Butterworth Q)
    let mut low_lp1 = context.create_biquad_filter();
    low_lp1.set_type(BiquadFilterType::Lowpass);
    low_lp1.frequency().set_value(FREQ_LOW);
    low_lp1.q().set_value(FRAC_1_SQRT_2);
    let mut low_lp2 = context.create_biquad_filter();
    low_lp2.set_type(BiquadFilterType::Lowpass);
    low_lp2.frequency().set_value(FREQ_LOW);
    low_lp2.q().set_value(FRAC_1_SQRT_2);
    let low_kill = context.create_gain(); // default gain = 1.0

    // Mid band: HPF @ 250 Hz + LPF @ 5 kHz (bandpass by cascade)
    let mut mid_hp = context.create_biquad_filter();
    mid_hp.set_type(BiquadFilterType::Highpass);
    mid_hp.frequency().set_value(FREQ_LOW);
    mid_hp.q().set_value(FRAC_1_SQRT_2);
    let mut mid_lp = context.create_biquad_filter();
    mid_lp.set_type(BiquadFilterType::Lowpass);
    mid_lp.frequency().set_value(FREQ_HIGH);
    mid_lp.q().set_value(FRAC_1_SQRT_2);
    let mid_kill = context.create_gain();

    // High band: 2× Butterworth HPF @ 5 kHz
    let mut high_hp1 = context.create_biquad_filter();
    high_hp1.set_type(BiquadFilterType::Highpass);
    high_hp1.frequency().set_value(FREQ_HIGH);
    high_hp1.q().set_value(FRAC_1_SQRT_2);
    let mut high_hp2 = context.create_biquad_filter();
    high_hp2.set_type(BiquadFilterType::Highpass);
    high_hp2.frequency().set_value(FREQ_HIGH);
    high_hp2.q().set_value(FRAC_1_SQRT_2);
    let high_kill = context.create_gain();

    // Sum bus (receives all three kill paths, connects onward to deck_gain)
    let eq_bus = context.create_gain();

    // Wire low path
    source.connect(&low_lp1);
    low_lp1.connect(&low_lp2);
    low_lp2.connect(&low_kill);
    low_kill.connect(&eq_bus);

    // Wire mid path
    source.connect(&mid_hp);
    mid_hp.connect(&mid_lp);
    mid_lp.connect(&mid_kill);
    mid_kill.connect(&eq_bus);

    // Wire high path
    source.connect(&high_hp1);
    high_hp1.connect(&high_hp2);
    high_hp2.connect(&high_kill);
    high_kill.connect(&eq_bus);

    // Connect eq_bus → deck gain
    eq_bus.connect(deck_gain);

    Self {
      low_kill,
      mid_kill,
      high_kill,
      _eq_bus: eq_bus,
      _low_lp1: low_lp1,
      _low_lp2: low_lp2,
      _mid_hp: mid_hp,
      _mid_lp: mid_lp,
      _high_hp1: high_hp1,
      _high_hp2: high_hp2,
    }
  }

  fn set_cuts(&self, state: EqCutState) {
    self.low_kill.gain().set_value(if state.low { 0.0 } else { 1.0 });
    self.mid_kill.gain().set_value(if state.mid { 0.0 } else { 1.0 });
    self.high_kill.gain().set_value(if state.high { 0.0 } else { 1.0 });
  }
}

struct WebAudioGraph {
  context: AudioContext,
  deck_a: BufferFeeder,
  deck_b: BufferFeeder,
  mic: BufferFeeder,
  cue: BufferFeeder,
  _deck_a_source: MediaStreamTrackAudioSourceNode,
  _deck_b_source: MediaStreamTrackAudioSourceNode,
  _mic_source: MediaStreamTrackAudioSourceNode,
  _cue_source: MediaStreamTrackAudioSourceNode,
  deck_a_eq: DeckEq,
  deck_b_eq: DeckEq,
  deck_a_gain: GainNode,
  deck_b_gain: GainNode,
  music_bus: GainNode,
  mic_gain: GainNode,
  _master_bus: GainNode,
  _main_splitter: ChannelSplitterNode,
  _cue_splitter: ChannelSplitterNode,
  _main_merger: ChannelMergerNode,
}

impl WebAudioGraph {
  fn new(io: &EngineIoConfig, sample_rate: f32) -> Result<Self> {
    let output_channels = io.output_channels.max(2) as usize;
    let context = AudioContext::new(AudioContextOptions {
      sample_rate: Some(sample_rate),
      sink_id: resolve_sink_id(io.output_device_name.as_deref()),
      latency_hint: AudioContextLatencyCategory::Playback,
      ..AudioContextOptions::default()
    });

    let (deck_a, deck_a_track) = BufferFeeder::new(2, sample_rate);
    let (deck_b, deck_b_track) = BufferFeeder::new(2, sample_rate);
    let (mic, mic_track) = BufferFeeder::new(2, sample_rate);
    let (cue, cue_track) = BufferFeeder::new(2, sample_rate);

    let deck_a_source = context.create_media_stream_track_source(&deck_a_track);
    let deck_b_source = context.create_media_stream_track_source(&deck_b_track);
    let mic_source = context.create_media_stream_track_source(&mic_track);
    let cue_source = context.create_media_stream_track_source(&cue_track);

    let deck_a_gain = context.create_gain();
    let deck_b_gain = context.create_gain();
    let music_bus = context.create_gain();
    let mic_gain = context.create_gain();
    let master_bus = context.create_gain();
    let main_splitter = context.create_channel_splitter(2);
    let cue_splitter = context.create_channel_splitter(2);
    let main_merger = context.create_channel_merger(output_channels);

    // EQ chains: source → [3-band parallel filter + kill] → eq_bus → deck_gain
    let deck_a_eq = DeckEq::new(&context, &deck_a_source, &deck_a_gain);
    let deck_b_eq = DeckEq::new(&context, &deck_b_source, &deck_b_gain);
    deck_a_gain.connect(&music_bus);
    deck_b_gain.connect(&music_bus);
    music_bus.connect(&master_bus);

    mic_source.connect(&mic_gain);
    mic_gain.connect(&master_bus);

    master_bus.connect(&main_splitter);
    cue_source.connect(&cue_splitter);
    route_stereo_to_merger(&main_splitter, &main_merger, io.main_channels);
    route_stereo_to_merger(&cue_splitter, &main_merger, io.cue_channels);
    main_merger.connect(&context.destination());

    Ok(Self {
      context,
      deck_a,
      deck_b,
      mic,
      cue,
      _deck_a_source: deck_a_source,
      _deck_b_source: deck_b_source,
      _mic_source: mic_source,
      _cue_source: cue_source,
      deck_a_eq,
      deck_b_eq,
      deck_a_gain,
      deck_b_gain,
      music_bus,
      mic_gain,
      _master_bus: master_bus,
      _main_splitter: main_splitter,
      _cue_splitter: cue_splitter,
      _main_merger: main_merger,
    })
  }

  fn set_mix_levels(&self, deck_a_gain: f32, deck_b_gain: f32, music_gain: f32, mic_gain: f32) {
    self.deck_a_gain.gain().set_value(deck_a_gain);
    self.deck_b_gain.gain().set_value(deck_b_gain);
    self.music_bus.gain().set_value(music_gain);
    self.mic_gain.gain().set_value(mic_gain);
  }

  fn set_deck_eq_cuts(&self, deck_a_eq: EqCutState, deck_b_eq: EqCutState) {
    self.deck_a_eq.set_cuts(deck_a_eq);
    self.deck_b_eq.set_cuts(deck_b_eq);
  }

  fn close(self) {
    self.context.close_sync();
  }
}

fn mix_and_route_with_gains(
  io: &EngineIoConfig,
  input: RenderInput<'_>,
  deck_a_gain: f32,
  deck_b_gain: f32,
) -> RenderOutput {
  let stereo = 2usize;
  let samples = input.frames * stereo;
  let deck_a = input.deck_a.unwrap_or(&[]);
  let deck_b = input.deck_b.unwrap_or(&[]);
  let mic = input.mic.unwrap_or(&[]);

  let mut mixed = vec![0.0f32; samples];
  let mut deck_a_peak = 0.0f32;
  let mut deck_b_peak = 0.0f32;
  let mut mic_peak = 0.0f32;

  let (music_attenuation, mic_gain) = if input.mic_enabled {
    (1.0 - input.talkover_ducking, input.mic_gain)
  } else {
    (1.0, 0.0)
  };

  for i in 0..samples {
    let a = deck_a.get(i).copied().unwrap_or(0.0);
    let b = deck_b.get(i).copied().unwrap_or(0.0);
    let m = mic.get(i).copied().unwrap_or(0.0);

    deck_a_peak = deck_a_peak.max((a * input.deck_a_gain).abs());
    deck_b_peak = deck_b_peak.max((b * input.deck_b_gain).abs());
    mic_peak = mic_peak.max(m.abs());

    let music = a * deck_a_gain + b * deck_b_gain;
    mixed[i] = music * music_attenuation + m * mic_gain;
  }

  let interleaved = route_main_and_cue(
    io,
    &mixed,
    input.frames,
    deck_a,
    deck_b,
    input.deck_a_cue,
    input.deck_b_cue,
  );

  RenderOutput {
    interleaved,
    deck_a_peak,
    deck_b_peak,
    mic_peak,
  }
}

fn interleaved_to_buffer(
  interleaved: &[f32],
  frames: usize,
  channels: usize,
  sample_rate: f32,
) -> AudioBuffer {
  let channel_count = channels.max(1);
  let mut data = vec![vec![0.0f32; frames]; channel_count];

  for frame in 0..frames {
    for channel in 0..channel_count {
      let sample = interleaved
        .get(frame * channel_count + channel)
        .copied()
        .unwrap_or_else(|| {
          if channel > 0 {
            data[0][frame]
          } else {
            0.0
          }
        });
      data[channel][frame] = sample;
    }
  }

  AudioBuffer::from(data, sample_rate)
}

fn route_stereo_to_merger(
  splitter: &ChannelSplitterNode,
  merger: &ChannelMergerNode,
  target_channels: [Option<u16>; 2],
) {
  let [left, right] = target_channels;
  if let Some(l) = left {
    splitter.connect_from_output_to_input(merger, 0, l as usize);
  }
  if let Some(r) = right {
    splitter.connect_from_output_to_input(merger, 1, r as usize);
  }
}

fn build_cue_mix(
  deck_a: &[f32],
  deck_b: &[f32],
  frames: usize,
  deck_a_cue: bool,
  deck_b_cue: bool,
) -> Vec<f32> {
  let mut out = vec![0.0f32; frames * 2];
  let mut sources = 0.0f32;
  if deck_a_cue {
    sources += 1.0;
  }
  if deck_b_cue {
    sources += 1.0;
  }
  if sources == 0.0 {
    return out;
  }

  for frame in 0..frames {
    let i = frame * 2;
    let a_l = deck_a.get(i).copied().unwrap_or(0.0);
    let a_r = deck_a.get(i + 1).copied().unwrap_or(a_l);
    let b_l = deck_b.get(i).copied().unwrap_or(0.0);
    let b_r = deck_b.get(i + 1).copied().unwrap_or(b_l);

    let l = (if deck_a_cue { a_l } else { 0.0 } + if deck_b_cue { b_l } else { 0.0 }) / sources;
    let r = (if deck_a_cue { a_r } else { 0.0 } + if deck_b_cue { b_r } else { 0.0 }) / sources;
    out[i] = l.clamp(-1.0, 1.0);
    out[i + 1] = r.clamp(-1.0, 1.0);
  }
  out
}

fn resolve_sink_id(device_name: Option<&str>) -> String {
  let Some(device_name) = device_name else {
    return String::new();
  };

  enumerate_devices_sync()
    .into_iter()
    .find(|device| device.kind() == MediaDeviceInfoKind::AudioOutput && device.label() == device_name)
    .map(|device| device.device_id().to_string())
    .unwrap_or_default()
}

fn route_main_and_cue(
  io: &EngineIoConfig,
  mix: &[f32],
  frames: usize,
  deck_a: &[f32],
  deck_b: &[f32],
  deck_a_cue: bool,
  deck_b_cue: bool,
) -> Vec<f32> {
  let out_ch = io.output_channels.max(2) as usize;
  let mut output = vec![0.0f32; frames * out_ch];
  let [main_l, main_r] = io.main_channels;
  let [cue_l, cue_r] = io.cue_channels;

  for frame in 0..frames {
    let i = frame * 2;
    let o = frame * out_ch;

    let main_left = mix.get(i).copied().unwrap_or(0.0);
    let main_right = mix.get(i + 1).copied().unwrap_or(main_left);
    let mono_main = (main_left + main_right) * 0.5;

    if let (Some(l), Some(r)) = (main_l, main_r) {
      if (l as usize) < out_ch {
        output[o + l as usize] = main_left;
      }
      if (r as usize) < out_ch {
        output[o + r as usize] = main_right;
      }
    } else if let Some(l) = main_l {
      if (l as usize) < out_ch {
        output[o + l as usize] = mono_main;
      }
    } else if let Some(r) = main_r {
      if (r as usize) < out_ch {
        output[o + r as usize] = mono_main;
      }
    }

    if cue_l.is_none() && cue_r.is_none() {
      continue;
    }

    let cue_left = (if deck_a_cue {
      deck_a.get(i).copied().unwrap_or(0.0)
    } else {
      0.0
    } + if deck_b_cue {
      deck_b.get(i).copied().unwrap_or(0.0)
    } else {
      0.0
    }) * 0.5;

    let cue_right = (if deck_a_cue {
      deck_a.get(i + 1).copied().unwrap_or(cue_left)
    } else {
      0.0
    } + if deck_b_cue {
      deck_b.get(i + 1).copied().unwrap_or(cue_left)
    } else {
      0.0
    }) * 0.5;

    let mono_cue = (cue_left + cue_right) * 0.5;

    if let Some(l) = cue_l {
      if (l as usize) < out_ch {
        output[o + l as usize] = if cue_r.is_some() { cue_left } else { mono_cue };
      }
    }
    if let Some(r) = cue_r {
      if (r as usize) < out_ch {
        output[o + r as usize] = if cue_l.is_some() { cue_right } else { mono_cue };
      }
    }
  }

  output.iter_mut().for_each(|sample| *sample = sample.clamp(-1.0, 1.0));
  output
}
