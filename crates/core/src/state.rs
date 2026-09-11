//! Plain data types describing what the UI shows and what preferences hold.
//!
//! These were `sujay_decks::ui_state`; they live here because the host
//! orchestration produces them and every UI (egui today, Swift next) consumes
//! them. All of them serialise, so a UI on the far side of an FFI boundary can
//! take them as JSON.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeckCueVisualState {
  pub label: String,
  /// Cue position as a fraction of the track (0..1).
  pub position: f32,
  /// Loop end as a fraction of the track (0..1), for loop cues.
  pub loop_end: Option<f32>,
  pub color_rgb: Option<[u8; 3]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
  pub rekordbox_cues: Vec<DeckCueVisualState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryTrackItem {
  pub id: String,
  pub title: String,
  pub artist: String,
  pub album: String,
  pub bpm: Option<f32>,
  pub duration_seconds: Option<f32>,
  pub rating: Option<i32>,
  pub tags: Option<String>,
  pub release_date: Option<String>,
  pub file_path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryPlaylistItem {
  pub id: String,
  pub name: String,
  pub parent_id: String,
  pub is_folder: bool,
  pub track_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryVisualState {
  pub source_label: String,
  pub tracks: Vec<LibraryTrackItem>,
  pub playlists: Vec<LibraryPlaylistItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
  pub name: String,
  pub max_output_channels: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
