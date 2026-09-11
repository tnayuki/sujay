//! Persisted preferences: JSON on disk, normalised against the audio device
//! list, and projected to the engine's device configuration and to the UI's
//! `PreferencesState`.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sujay_audio::engine_core::{list_output_devices, DeviceConfigCore};

use crate::state::{AudioDeviceInfo, PreferencesState};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppPreferences {
  #[serde(default)]
  pub audio_device_id: Option<String>,
  #[serde(default = "default_main_channels")]
  pub main_channels: [Option<i32>; 2],
  #[serde(default = "default_cue_channels")]
  pub cue_channels: [Option<i32>; 2],
  #[serde(default = "default_recording_directory")]
  pub recording_directory: String,
  #[serde(default = "default_recording_auto_create_directory")]
  pub recording_auto_create_directory: bool,
  #[serde(default = "default_recording_naming_strategy")]
  pub recording_naming_strategy: String,
  #[serde(default = "default_recording_format")]
  pub recording_format: String,
  #[serde(default = "default_osc_enabled")]
  pub osc_enabled: bool,
  #[serde(default = "default_osc_host")]
  pub osc_host: String,
  #[serde(default = "default_osc_port")]
  pub osc_port: u16,
}

impl Default for AppPreferences {
  fn default() -> Self {
    Self {
      audio_device_id: None,
      main_channels: default_main_channels(),
      cue_channels: default_cue_channels(),
      recording_directory: default_recording_directory(),
      recording_auto_create_directory: default_recording_auto_create_directory(),
      recording_naming_strategy: default_recording_naming_strategy(),
      recording_format: default_recording_format(),
      osc_enabled: default_osc_enabled(),
      osc_host: default_osc_host(),
      osc_port: default_osc_port(),
    }
  }
}

fn default_main_channels() -> [Option<i32>; 2] {
  [Some(0), Some(1)]
}

fn default_cue_channels() -> [Option<i32>; 2] {
  [None, None]
}

fn default_recording_directory() -> String {
  dirs::audio_dir()
    .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
    .join("Sujay Recordings")
    .to_string_lossy()
    .to_string()
}

fn default_recording_format() -> String {
  "wav".to_owned()
}

fn default_recording_auto_create_directory() -> bool {
  true
}

fn default_recording_naming_strategy() -> String {
  "timestamp".to_owned()
}

fn default_osc_enabled() -> bool {
  false
}

fn default_osc_host() -> String {
  "127.0.0.1".to_owned()
}

fn default_osc_port() -> u16 {
  9000
}

impl AppPreferences {
  /// Clamp channels to the selected device, drop duplicates, and fill any
  /// empty string/enum field with its default.
  pub fn normalize(&mut self, audio_devices: &[AudioDeviceInfo]) {
    let selected_max = self
      .audio_device_id
      .as_ref()
      .and_then(|id| audio_devices.iter().find(|d| &d.name == id))
      .map(|d| d.max_output_channels as i32)
      .unwrap_or(2)
      .max(2);

    let mut used = HashSet::new();
    for idx in 0..2 {
      if let Some(ch) = self.main_channels[idx] {
        if ch < 0 || ch >= selected_max || !used.insert(ch) {
          self.main_channels[idx] = None;
        }
      }
    }
    for idx in 0..2 {
      if let Some(ch) = self.cue_channels[idx] {
        if ch < 0 || ch >= selected_max || !used.insert(ch) {
          self.cue_channels[idx] = None;
        }
      }
    }

    if self.main_channels[0].is_none() && self.main_channels[1].is_none() {
      self.main_channels[0] = Some(0);
      if selected_max > 1 {
        self.main_channels[1] = Some(1);
      }
    }

    if self.recording_directory.trim().is_empty() {
      self.recording_directory = default_recording_directory();
    }
    if self.recording_naming_strategy != "timestamp"
      && self.recording_naming_strategy != "sequential"
    {
      self.recording_naming_strategy = default_recording_naming_strategy();
    }
    if self.recording_format != "wav" && self.recording_format != "ogg" {
      self.recording_format = default_recording_format();
    }
    if self.osc_host.trim().is_empty() {
      self.osc_host = default_osc_host();
    }
    if self.osc_port == 0 {
      self.osc_port = default_osc_port();
    }
  }

  /// Overwrite from a UI-edited state, then normalise.
  pub fn apply_state(&mut self, state: PreferencesState, audio_devices: &[AudioDeviceInfo]) {
    self.audio_device_id = state.audio_device_id;
    self.main_channels = state.main_channels;
    self.cue_channels = state.cue_channels;
    self.recording_directory = state.recording_directory;
    self.recording_auto_create_directory = state.recording_auto_create_directory;
    self.recording_naming_strategy = state.recording_naming_strategy;
    self.recording_format = state.recording_format;
    self.osc_enabled = state.osc_enabled;
    self.osc_host = state.osc_host;
    self.osc_port = state.osc_port;
    self.normalize(audio_devices);
  }

  pub fn to_state(&self, audio_devices: &[AudioDeviceInfo]) -> PreferencesState {
    PreferencesState {
      audio_device_id: self.audio_device_id.clone(),
      audio_devices: audio_devices.to_vec(),
      main_channels: self.main_channels,
      cue_channels: self.cue_channels,
      recording_directory: self.recording_directory.clone(),
      recording_auto_create_directory: self.recording_auto_create_directory,
      recording_naming_strategy: self.recording_naming_strategy.clone(),
      recording_format: self.recording_format.clone(),
      osc_enabled: self.osc_enabled,
      osc_host: self.osc_host.clone(),
      osc_port: self.osc_port,
    }
  }

  pub fn device_config(&self) -> DeviceConfigCore {
    DeviceConfigCore {
      device_id: self.audio_device_id.clone(),
      main_channels: Some(self.main_channels.iter().map(|v| v.unwrap_or(-1)).collect()),
      cue_channels: Some(self.cue_channels.iter().map(|v| v.unwrap_or(-1)).collect()),
    }
  }

  /// Pick the next free recording file under the configured directory.
  pub fn prepare_recording_path(&self) -> Result<PathBuf, String> {
    let rec_dir = PathBuf::from(&self.recording_directory);
    if !rec_dir.is_absolute() {
      return Err("recording directory must be an absolute path".to_owned());
    }

    if !rec_dir.exists() {
      if !self.recording_auto_create_directory {
        return Err(format!(
          "recording directory not found: {}",
          rec_dir.display()
        ));
      }
      fs::create_dir_all(&rec_dir).map_err(|e| e.to_string())?;
    }

    let ext = recording_extension(&self.recording_format);
    if self.recording_naming_strategy == "sequential" {
      for index in 1..=9999 {
        let path = rec_dir.join(format!("{:04}.{}", index, ext));
        if !path.exists() {
          return Ok(path);
        }
      }
      return Err("unable to allocate recording filename".to_owned());
    }

    let ts = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let base = format!("sujay_{}", ts);
    for suffix in 0..=999 {
      let name = if suffix == 0 {
        format!("{}.{}", base, ext)
      } else {
        format!("{}_{}.{}", base, suffix, ext)
      };
      let path = rec_dir.join(name);
      if !path.exists() {
        return Ok(path);
      }
    }
    Err("unable to allocate timestamp recording filename".to_owned())
  }

  pub fn load(path: &Path) -> AppPreferences {
    fs::read_to_string(path)
      .ok()
      .and_then(|json| serde_json::from_str(&json).ok())
      .unwrap_or_default()
  }

  /// Write atomically: temp file, fsync, rename.
  pub fn save(&self, path: &Path) -> Result<(), String> {
    let parent = path
      .parent()
      .ok_or_else(|| "Invalid settings path".to_owned())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;

    {
      let mut file = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
      file.write_all(&json).map_err(|e| e.to_string())?;
      file.sync_all().map_err(|e| e.to_string())?;
    }

    fs::rename(&tmp_path, path).map_err(|e| e.to_string())
  }
}

fn recording_extension(format: &str) -> &'static str {
  if format == "ogg" {
    "ogg"
  } else {
    "wav"
  }
}

pub fn settings_file_path() -> PathBuf {
  dirs::data_local_dir()
    .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
    .join("Sujay")
    .join("settings.json")
}

pub fn available_audio_devices() -> Vec<AudioDeviceInfo> {
  list_output_devices()
    .unwrap_or_default()
    .into_iter()
    .map(|(name, max_output_channels)| AudioDeviceInfo {
      name,
      max_output_channels,
    })
    .collect()
}
