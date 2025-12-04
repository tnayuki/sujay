#![deny(clippy::all)]

use cpal::traits::{DeviceTrait, HostTrait};
use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object)]
pub struct AudioDeviceInfo {
  pub name: String,
  pub max_input_channels: u32,
  pub max_output_channels: u32,
  pub default_sample_rate: Option<f64>,
}

/// Returns the crate version so JS can verify the native module loaded correctly.
#[napi]
pub fn addon_version() -> String {
  env!("CARGO_PKG_VERSION").to_string()
}

#[napi]
pub fn list_audio_devices() -> Result<Vec<AudioDeviceInfo>> {
  let host = cpal::default_host();
  let mut devices = Vec::new();
  for device in host.devices().map_err(map_err)? {
    let name = device.name().unwrap_or_else(|_| "Unknown".to_string());

    let max_input_channels = device
      .supported_input_configs()
      .ok()
      .and_then(|configs| {
        configs
          .max_by_key(|cfg| cfg.channels())
          .map(|cfg| cfg.channels())
      })
      .unwrap_or(0);

    let max_output_channels = device
      .supported_output_configs()
      .ok()
      .and_then(|configs| {
        configs
          .max_by_key(|cfg| cfg.channels())
          .map(|cfg| cfg.channels())
      })
      .unwrap_or(0);

    let default_sample_rate = device
      .default_output_config()
      .map(|cfg| cfg.sample_rate().0 as f64)
      .ok();

    devices.push(AudioDeviceInfo {
      name,
      max_input_channels: max_input_channels as u32,
      max_output_channels: max_output_channels as u32,
      default_sample_rate,
    });
  }

  Ok(devices)
}

fn map_err<E: ToString>(err: E) -> Error {
  Error::from_reason(err.to_string())
}

// ============================================================================
// Audio Engine - Core DJ mixing engine
// ============================================================================

mod beat_detector;

// ============================================================================
// Beat Detection - Multi-feature beat tracker (Zapata et al. 2014)
// ============================================================================

#[napi(object)]
pub struct BeatDetectionResultJs {
  /// Detected BPM
  pub bpm: f64,
  /// Beat positions in seconds
  pub beats: Vec<f64>,
  /// Confidence score (0-1)
  pub confidence: f64,
}

/// Detect BPM and beat positions from mono audio data.
/// Based on: J. Zapata, M. Davies and E. Gómez, "Multi-feature beat tracker,"
/// IEEE/ACM Transactions on Audio, Speech and Language Processing, 22(4), 816-825, 2014
#[napi]
pub fn detect_beats(audio: Float32Array, sample_rate: f64) -> Option<BeatDetectionResultJs> {
  let mut detector = beat_detector::BeatDetector::new(sample_rate as f32);
  let result = detector.detect(audio.as_ref())?;

  Some(BeatDetectionResultJs {
    bpm: result.bpm as f64,
    beats: result.beats.iter().map(|&b| b as f64).collect(),
    confidence: result.confidence as f64,
  })
}

mod audio_engine;
mod decoder;
mod eq_processor;
mod recorder;
pub use audio_engine::*;
pub use decoder::*;
pub use recorder::*;
