#[allow(dead_code)]
#[derive(Clone, Default, PartialEq)]
pub struct TitlebarState {
  /// Current wall-clock time as "HH:MM:SS".
  pub time_text: String,
  /// CPU usage in percent (0–100).
  pub cpu_percent: f32,
  /// Memory usage in megabytes.
  pub mem_mb: u64,
  /// Whether the microphone input is available on the current device.
  pub mic_available: bool,
  /// Whether the microphone is currently enabled.
  pub mic_enabled: bool,
  /// Microphone peak level (0.0–1.0).
  pub mic_peak: f32,
  /// Whether a recording session is currently active.
  pub is_recording: bool,
  /// Elapsed recording time in seconds (0 when not recording).
  pub rec_elapsed_secs: u32,
}

#[allow(dead_code)]
#[derive(Clone, Default, PartialEq)]
pub struct DeckConsoleVisualState {
  pub title: String,
  pub time_text: String,
  pub bpm_text: String,
  pub bpm: f32,
  pub playing: bool,
  pub loop_enabled: bool,
  pub loop_beats: f32,
  pub loop_start: f32,
  pub loop_end: f32,
  pub cue_enabled: bool,
  pub eq_low: bool,
  pub eq_mid: bool,
  pub eq_high: bool,
  pub gain: f32,
  pub peak: f32,
}

#[allow(dead_code)]
#[derive(Clone, PartialEq)]
pub struct ConsoleVisualState {
  pub titlebar: TitlebarState,
  pub deck_a: DeckConsoleVisualState,
  pub deck_b: DeckConsoleVisualState,
  pub master_tempo: f32,
  pub crossfader: f32,
}

impl Default for ConsoleVisualState {
  fn default() -> Self {
    Self {
      titlebar: TitlebarState::default(),
      deck_a: DeckConsoleVisualState::default(),
      deck_b: DeckConsoleVisualState::default(),
      master_tempo: 130.0,
      crossfader: 0.5,
    }
  }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioDeviceInfo {
  pub name: String,
  pub max_output_channels: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreferencesState {
  pub audio_device_id: Option<String>,
  pub audio_devices: Vec<AudioDeviceInfo>,
  pub main_channels: [Option<i32>; 2],
  pub cue_channels: [Option<i32>; 2],
  pub recording_directory: String,
  pub recording_auto_create_directory: bool,
  pub recording_naming_strategy: String,
  pub recording_format: String,
  pub osc_enabled: bool,
  pub osc_host: String,
  pub osc_port: u16,
}

impl Default for PreferencesState {
  fn default() -> Self {
    Self {
      audio_device_id: None,
      audio_devices: vec![],
      main_channels: [Some(0), Some(1)],
      cue_channels: [None, None],
      recording_directory: String::new(),
      recording_auto_create_directory: true,
      recording_naming_strategy: "timestamp".to_owned(),
      recording_format: "wav".to_owned(),
      osc_enabled: false,
      osc_host: "127.0.0.1".to_owned(),
      osc_port: 9000,
    }
  }
}
