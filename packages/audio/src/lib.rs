#![deny(clippy::all)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

const DEFAULT_SAMPLE_RATE: u32 = 44_100;
const DEFAULT_CHANNELS: u16 = 2;
const DEFAULT_INPUT_CHANNELS: u16 = 1;

type SharedQueue = Arc<Mutex<VecDeque<f32>>>;

#[napi(object)]
pub struct AudioDeviceInfo {
  pub id: String,
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
  for (idx, device) in host.devices().map_err(map_err)?.enumerate() {
    let id = idx.to_string();
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
      id,
      name,
      max_input_channels: max_input_channels as u32,
      max_output_channels: max_output_channels as u32,
      default_sample_rate,
    });
  }

  Ok(devices)
}

#[napi]
pub struct AudioOutputStream {
  queue: SharedQueue,
  is_closed: Arc<Mutex<bool>>,
  _thread_handle: Option<JoinHandle<()>>,
}

#[napi]
impl AudioOutputStream {
  #[napi(constructor)]
  pub fn new(
    device_id: Option<String>,
    channels: Option<u16>,
    sample_rate: Option<u32>,
  ) -> Result<Self> {
    let host = cpal::default_host();
    let mut selected = None;
    for (idx, device) in host.devices().map_err(map_err)?.enumerate() {
      let candidate_id = idx.to_string();
      if device_id
        .as_ref()
        .map_or(idx == 0, |id| *id == candidate_id)
      {
        selected = Some(device);
        break;
      }
    }

    let device = selected.ok_or_else(|| Error::from_reason("Audio device not found"))?;
    let config = device.default_output_config().map_err(map_err)?;

    if config.sample_format() != SampleFormat::F32 {
      return Err(Error::from_reason("Device does not support f32 output"));
    }

    let mut final_config = config.config();
    final_config.channels = channels.unwrap_or(DEFAULT_CHANNELS);
    final_config.sample_rate.0 = sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE);

    let queue: SharedQueue = Arc::new(Mutex::new(VecDeque::new()));
    let queue_clone = Arc::clone(&queue);
    let is_closed = Arc::new(Mutex::new(false));
    let is_closed_clone = Arc::clone(&is_closed);

    // Spawn dedicated thread to keep stream alive
    let thread_handle = thread::spawn(move || {
      let stream = device
        .build_output_stream(
          &final_config,
          move |data: &mut [f32], _| {
            fill_from_queue(data, &queue_clone);
          },
          move |err| eprintln!("[sujay-audio] Output stream error: {err}"),
          None,
        )
        .expect("Failed to build stream");

      stream.play().expect("Failed to start stream");

      // Keep stream alive until closed
      loop {
        thread::sleep(Duration::from_millis(100));
        if *is_closed_clone.lock().unwrap() {
          break;
        }
      }
    });

    Ok(Self {
      queue,
      is_closed,
      _thread_handle: Some(thread_handle),
    })
  }

  /// Write PCM frames (interleaved Float32) into the output queue.
  #[napi]
  pub fn write(&self, chunk: Float32Array) -> Result<()> {
    if *self.is_closed.lock().unwrap() {
      return Err(Error::from_reason("Stream is closed"));
    }
    let mut queue = self
      .queue
      .lock()
      .map_err(|_| Error::from_reason("queue poisoned"))?;
    queue.extend(chunk.as_ref());
    Ok(())
  }

  /// Returns the number of samples currently in the queue.
  #[napi]
  pub fn queue_size(&self) -> Result<u32> {
    let queue = self
      .queue
      .lock()
      .map_err(|_| Error::from_reason("queue poisoned"))?;
    Ok(queue.len() as u32)
  }

  /// Clears any queued audio data, useful when seeking.
  #[napi]
  pub fn clear(&self) -> Result<()> {
    let mut queue = self
      .queue
      .lock()
      .map_err(|_| Error::from_reason("queue poisoned"))?;
    queue.clear();
    Ok(())
  }

  /// Stops playback and releases the device handle.
  #[napi]
  pub fn close(&mut self) -> Result<()> {
    let mut closed = self.is_closed.lock().unwrap();
    if *closed {
      return Ok(());
    }

    *closed = true;
    drop(closed); // Release lock before joining thread

    Ok(())
  }
}

impl Drop for AudioOutputStream {
  fn drop(&mut self) {
    let mut closed = self.is_closed.lock().unwrap();
    *closed = true;
  }
}

#[napi]
pub struct AudioInputStream {
  is_closed: Arc<Mutex<bool>>,
  _thread_handle: Option<JoinHandle<()>>,
}

#[napi]
impl AudioInputStream {
  /// Create a new audio input stream.
  /// The callback receives Float32Array chunks of audio data.
  #[napi(constructor)]
  pub fn new(
    device_id: Option<String>,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    #[napi(ts_arg_type = "(data: Float32Array) => void")] callback: Function<Float32Array, ()>,
  ) -> Result<Self> {
    let host = cpal::default_host();
    let mut selected = None;
    for (idx, device) in host.devices().map_err(map_err)?.enumerate() {
      let candidate_id = idx.to_string();
      if device_id
        .as_ref()
        .map_or(idx == 0, |id| *id == candidate_id)
      {
        selected = Some(device);
        break;
      }
    }

    let device = selected.ok_or_else(|| Error::from_reason("Audio device not found"))?;
    let config = device.default_input_config().map_err(map_err)?;

    if config.sample_format() != SampleFormat::F32 {
      return Err(Error::from_reason("Device does not support f32 input"));
    }

    let mut final_config = config.config();
    final_config.channels = channels.unwrap_or(DEFAULT_INPUT_CHANNELS);
    final_config.sample_rate.0 = sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE);

    let is_closed = Arc::new(Mutex::new(false));
    let is_closed_clone = Arc::clone(&is_closed);

    // Create threadsafe function for callback
    // build_callback returns Vec<T> which becomes the callback arguments
    // Since callback is (data: Float32Array) => void, we return vec![Float32Array]
    let tsfn = callback
      .build_threadsafe_function()
      .callee_handled::<false>()
      .build_callback(|ctx| {
        let samples: Vec<f32> = ctx.value;
        // Return single-element vec - this becomes the single argument to callback
        Ok(vec![Float32Array::new(samples)])
      })?;

    // Spawn dedicated thread to keep stream alive
    let thread_handle = thread::spawn(move || {
      let stream = device
        .build_input_stream(
          &final_config,
          move |data: &[f32], _| {
            // Clone data and send via threadsafe function
            let samples = data.to_vec();
            tsfn.call(samples, ThreadsafeFunctionCallMode::NonBlocking);
          },
          move |err| eprintln!("[sujay-audio] Input stream error: {err}"),
          None,
        )
        .expect("Failed to build input stream");

      stream.play().expect("Failed to start input stream");

      // Keep stream alive until closed
      loop {
        thread::sleep(Duration::from_millis(100));
        if *is_closed_clone.lock().unwrap() {
          break;
        }
      }
    });

    Ok(Self {
      is_closed,
      _thread_handle: Some(thread_handle),
    })
  }

  /// Stops input capture and releases the device handle.
  #[napi]
  pub fn close(&mut self) -> Result<()> {
    let mut closed = self.is_closed.lock().unwrap();
    if *closed {
      return Ok(());
    }

    *closed = true;
    Ok(())
  }
}

impl Drop for AudioInputStream {
  fn drop(&mut self) {
    let mut closed = self.is_closed.lock().unwrap();
    *closed = true;
  }
}

fn fill_from_queue(buffer: &mut [f32], queue: &SharedQueue) {
  if let Ok(mut queue) = queue.lock() {
    for sample in buffer.iter_mut() {
      *sample = queue.pop_front().unwrap_or(0.0);
    }
  } else {
    buffer.fill(0.0);
  }
}

fn map_err<E: ToString>(err: E) -> Error {
  Error::from_reason(err.to_string())
}
