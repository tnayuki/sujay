//! 3-Band EQ Processor with Kill Switches
//!
//! Implements DJ-style frequency isolation matching Pioneer DJM mixers.
//! Frequency bands:
//! - LOW: Below 250 Hz (lowpass)
//! - MID: 250 Hz to 5 kHz (bandpass)
//! - HIGH: Above 5 kHz (highpass)
//!
//! Uses overlapping filters for smooth transitions, similar to analog DJ mixers.

use std::f32::consts::PI;

const SAMPLE_RATE: f32 = 44100.0;

// DJ mixer style frequency bands (overlapping for smooth transitions)
const FREQ_LOW: f32 = 250.0;
const FREQ_MID_LOW: f32 = 250.0;
const FREQ_MID_HIGH: f32 = 5000.0;
const FREQ_HIGH: f32 = 5000.0;

/// Biquad filter coefficients (Direct Form I)
/// Transfer function: H(z) = (b0 + b1*z^-1 + b2*z^-2) / (1 + a1*z^-1 + a2*z^-2)
#[derive(Clone, Copy, Default)]
struct BiquadCoefficients {
  b0: f32,
  b1: f32,
  b2: f32,
  a1: f32,
  a2: f32,
}

/// Biquad filter state for one channel
#[derive(Default, Clone)]
struct BiquadFilterChannel {
  x1: f32, // Input delayed by 1 sample
  x2: f32, // Input delayed by 2 samples
  y1: f32, // Output delayed by 1 sample
  y2: f32, // Output delayed by 2 samples
}

impl BiquadFilterChannel {
  /// Process one sample through the biquad filter (Direct Form I)
  #[inline]
  fn process(&mut self, input: f32, coeffs: &BiquadCoefficients) -> f32 {
    let output = coeffs.b0 * input + coeffs.b1 * self.x1 + coeffs.b2 * self.x2
      - coeffs.a1 * self.y1
      - coeffs.a2 * self.y2;

    // Update delay line
    self.x2 = self.x1;
    self.x1 = input;
    self.y2 = self.y1;
    self.y1 = output;

    output
  }
}

/// Stereo biquad filter
#[derive(Default, Clone)]
struct BiquadFilter {
  left: BiquadFilterChannel,
  right: BiquadFilterChannel,
}

impl BiquadFilter {
  /// Process stereo interleaved buffer in-place
  fn process_interleaved(
    &mut self,
    buffer: &mut [f32],
    frames: usize,
    coeffs: &BiquadCoefficients,
  ) {
    for i in 0..frames {
      let left_idx = i * 2;
      let right_idx = i * 2 + 1;
      buffer[left_idx] = self.left.process(buffer[left_idx], coeffs);
      buffer[right_idx] = self.right.process(buffer[right_idx], coeffs);
    }
  }
}

/// Calculate 2nd-order Butterworth lowpass filter coefficients
fn calculate_butterworth_lowpass(fc: f32, sample_rate: f32) -> BiquadCoefficients {
  let q = 0.7071067811865476_f32; // 1/sqrt(2) for Butterworth

  let w0 = 2.0 * PI * fc / sample_rate;
  let cos_w0 = w0.cos();
  let sin_w0 = w0.sin();
  let alpha = sin_w0 / (2.0 * q);

  let a0 = 1.0 + alpha;
  BiquadCoefficients {
    b0: (1.0 - cos_w0) / 2.0 / a0,
    b1: (1.0 - cos_w0) / a0,
    b2: (1.0 - cos_w0) / 2.0 / a0,
    a1: -2.0 * cos_w0 / a0,
    a2: (1.0 - alpha) / a0,
  }
}

/// Calculate 2nd-order Butterworth highpass filter coefficients
fn calculate_butterworth_highpass(fc: f32, sample_rate: f32) -> BiquadCoefficients {
  let q = 0.7071067811865476_f32; // 1/sqrt(2) for Butterworth

  let w0 = 2.0 * PI * fc / sample_rate;
  let cos_w0 = w0.cos();
  let sin_w0 = w0.sin();
  let alpha = sin_w0 / (2.0 * q);

  let a0 = 1.0 + alpha;
  BiquadCoefficients {
    b0: (1.0 + cos_w0) / 2.0 / a0,
    b1: -(1.0 + cos_w0) / a0,
    b2: (1.0 + cos_w0) / 2.0 / a0,
    a1: -2.0 * cos_w0 / a0,
    a2: (1.0 - alpha) / a0,
  }
}

/// EQ cut state (kill switches)
#[derive(Clone, Copy, Default)]
pub struct EqCutState {
  pub low: bool,
  pub mid: bool,
  pub high: bool,
}

/// DJ-style 3-band EQ with overlapping filters
///
/// Each band uses independent filters for smooth, musical response
pub struct EqProcessor {
  // Low band: 2x Butterworth LPF at 250Hz
  low_filter1: BiquadFilter,
  low_filter2: BiquadFilter,
  low_coeffs: BiquadCoefficients,

  // Mid band: Bandpass 250Hz to 5kHz (HPF + LPF)
  mid_filter_low1: BiquadFilter,
  mid_filter_low2: BiquadFilter,
  mid_filter_high1: BiquadFilter,
  mid_filter_high2: BiquadFilter,
  mid_coeffs_low: BiquadCoefficients,
  mid_coeffs_high: BiquadCoefficients,

  // High band: 2x Butterworth HPF at 5kHz
  high_filter1: BiquadFilter,
  high_filter2: BiquadFilter,
  high_coeffs: BiquadCoefficients,

  // Kill states
  cut_state: EqCutState,

  // Temporary buffers for band processing
  low_buffer: Vec<f32>,
  mid_buffer: Vec<f32>,
  high_buffer: Vec<f32>,
}

impl EqProcessor {
  pub fn new(max_frames: usize) -> Self {
    // Low band: 2x Butterworth LPF at 250Hz
    let low_coeffs = calculate_butterworth_lowpass(FREQ_LOW, SAMPLE_RATE);

    // Mid band: Bandpass created by HPF (250Hz) + LPF (5kHz)
    let mid_coeffs_low = calculate_butterworth_highpass(FREQ_MID_LOW, SAMPLE_RATE);
    let mid_coeffs_high = calculate_butterworth_lowpass(FREQ_MID_HIGH, SAMPLE_RATE);

    // High band: 2x Butterworth HPF at 5kHz
    let high_coeffs = calculate_butterworth_highpass(FREQ_HIGH, SAMPLE_RATE);

    Self {
      low_filter1: BiquadFilter::default(),
      low_filter2: BiquadFilter::default(),
      low_coeffs,

      mid_filter_low1: BiquadFilter::default(),
      mid_filter_low2: BiquadFilter::default(),
      mid_filter_high1: BiquadFilter::default(),
      mid_filter_high2: BiquadFilter::default(),
      mid_coeffs_low,
      mid_coeffs_high,

      high_filter1: BiquadFilter::default(),
      high_filter2: BiquadFilter::default(),
      high_coeffs,

      cut_state: EqCutState::default(),

      low_buffer: vec![0.0; max_frames * 2],
      mid_buffer: vec![0.0; max_frames * 2],
      high_buffer: vec![0.0; max_frames * 2],
    }
  }

  /// Set kill state for a specific band
  pub fn set_cut(&mut self, band: EqBand, enabled: bool) {
    match band {
      EqBand::Low => self.cut_state.low = enabled,
      EqBand::Mid => self.cut_state.mid = enabled,
      EqBand::High => self.cut_state.high = enabled,
    }
  }

  /// Get current cut state
  pub fn get_cut_state(&self) -> EqCutState {
    self.cut_state
  }

  /// Process audio buffer with 3-band EQ and kill switches
  /// Uses independent overlapping filters for each band
  pub fn process(&mut self, buffer: &mut [f32], frames: usize) {
    let EqCutState { low, mid, high } = self.cut_state;

    // Optimization: bypass EQ if all bands are enabled (no kills active)
    if !low && !mid && !high {
      return;
    }

    // Optimization: complete silence if all bands are killed
    if low && mid && high {
      buffer[..frames * 2].fill(0.0);
      return;
    }

    let samples = frames * 2;

    // Copy input to all band buffers
    self.low_buffer[..samples].copy_from_slice(&buffer[..samples]);
    self.mid_buffer[..samples].copy_from_slice(&buffer[..samples]);
    self.high_buffer[..samples].copy_from_slice(&buffer[..samples]);

    // Apply filters to each band independently
    // Low: 2x LPF at 250Hz
    self
      .low_filter1
      .process_interleaved(&mut self.low_buffer, frames, &self.low_coeffs);
    self
      .low_filter2
      .process_interleaved(&mut self.low_buffer, frames, &self.low_coeffs);

    // Mid: HPF at 250Hz then LPF at 5kHz (creates bandpass)
    self
      .mid_filter_low1
      .process_interleaved(&mut self.mid_buffer, frames, &self.mid_coeffs_low);
    self
      .mid_filter_low2
      .process_interleaved(&mut self.mid_buffer, frames, &self.mid_coeffs_low);
    self
      .mid_filter_high1
      .process_interleaved(&mut self.mid_buffer, frames, &self.mid_coeffs_high);
    self
      .mid_filter_high2
      .process_interleaved(&mut self.mid_buffer, frames, &self.mid_coeffs_high);

    // High: 2x HPF at 5kHz
    self
      .high_filter1
      .process_interleaved(&mut self.high_buffer, frames, &self.high_coeffs);
    self
      .high_filter2
      .process_interleaved(&mut self.high_buffer, frames, &self.high_coeffs);

    // Mix bands with kill switches applied
    for i in 0..samples {
      buffer[i] = if low { 0.0 } else { self.low_buffer[i] }
        + if mid { 0.0 } else { self.mid_buffer[i] }
        + if high { 0.0 } else { self.high_buffer[i] };
    }
  }
}

/// EQ band identifiers
#[derive(Clone, Copy, Debug)]
pub enum EqBand {
  Low,
  Mid,
  High,
}
