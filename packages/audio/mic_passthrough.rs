//! Minimal microphone passthrough test using cpal
//! Tests if audio input -> output works without distortion

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

const RING_BUFFER_SIZE: usize = 44100 / 10 * 2; // ~100ms stereo buffer

fn main() {
  let host = cpal::default_host();

  // Find M4 device (use same device for both input and output)
  let mut m4_device = None;

  if let Ok(devices) = host.devices() {
    for device in devices {
      if let Ok(name) = device.name() {
        if name.contains("M4") {
          println!("Found M4 device: {}", name);
          m4_device = Some(device);
          break;
        }
      }
    }
  }

  let device = m4_device.expect("M4 device not found");

  println!("Using device: {:?}", device.name().unwrap_or_default());

  // Mono input config (like DJ app)
  let input_config = cpal::StreamConfig {
    channels: 1,
    sample_rate: cpal::SampleRate(44100),
    buffer_size: cpal::BufferSize::Default,
  };

  // 4-channel output config (using channels 3/4)
  let output_config = cpal::StreamConfig {
    channels: 4,
    sample_rate: cpal::SampleRate(44100),
    buffer_size: cpal::BufferSize::Default,
  };

  // Shared ring buffer (stereo samples)
  let ring_buffer = Arc::new(Mutex::new(RingBuffer::new()));

  let ring_for_input = Arc::clone(&ring_buffer);
  let ring_for_output = Arc::clone(&ring_buffer);

  // Build input stream (mono) from same device
  let input_stream = device
    .build_input_stream(
      &input_config,
      move |data: &[f32], _| {
        let mut ring = ring_for_input.lock().unwrap();
        // Write mono as stereo
        for &sample in data {
          ring.write(sample, sample);
        }
      },
      move |err| eprintln!("Input error: {err}"),
      None,
    )
    .expect("Failed to build input stream");

  // Build output stream (4ch, output to channels 3/4) from same device
  let output_stream = device
    .build_output_stream(
      &output_config,
      move |output: &mut [f32], _| {
        let mut ring = ring_for_output.lock().unwrap();
        // Output is interleaved: [ch0, ch1, ch2, ch3, ch0, ch1, ch2, ch3, ...]
        for chunk in output.chunks_mut(4) {
          let (left, right) = ring.read();
          chunk[0] = 0.0; // Ch 1 (silent)
          chunk[1] = 0.0; // Ch 2 (silent)
          chunk[2] = left; // Ch 3
          chunk[3] = right; // Ch 4
        }
      },
      move |err| eprintln!("Output error: {err}"),
      None,
    )
    .expect("Failed to build output stream");

  input_stream.play().unwrap();
  output_stream.play().unwrap();

  println!("Passthrough running (mono->ch3/4). Press Ctrl+C to stop.");
  loop {
    std::thread::sleep(std::time::Duration::from_secs(1));
  }
}

/// Simple ring buffer for stereo audio
struct RingBuffer {
  buffer: Vec<f32>,
  write_pos: usize,
  read_pos: usize,
}

impl RingBuffer {
  fn new() -> Self {
    Self {
      buffer: vec![0.0; RING_BUFFER_SIZE],
      write_pos: 0,
      read_pos: 0,
    }
  }

  fn write(&mut self, left: f32, right: f32) {
    let idx = (self.write_pos % (RING_BUFFER_SIZE / 2)) * 2;
    self.buffer[idx] = left;
    self.buffer[idx + 1] = right;
    self.write_pos += 1;
  }

  fn read(&mut self) -> (f32, f32) {
    // Check if data is available
    if self.write_pos <= self.read_pos {
      return (0.0, 0.0); // No data available
    }

    let idx = (self.read_pos % (RING_BUFFER_SIZE / 2)) * 2;
    let left = self.buffer[idx];
    let right = self.buffer[idx + 1];
    self.read_pos += 1;
    (left, right)
  }
}
