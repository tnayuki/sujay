/**
 * Biquad Filter Implementation
 * Based on RBJ Audio EQ Cookbook
 * https://webaudio.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html
 */

/**
 * Biquad filter coefficients (Direct Form I)
 * Transfer function: H(z) = (b0 + b1*z^-1 + b2*z^-2) / (1 + a1*z^-1 + a2*z^-2)
 */
export interface BiquadCoefficients {
  b0: number;
  b1: number;
  b2: number;
  a1: number; // Note: a0 is normalized to 1.0
  a2: number;
}

/**
 * Biquad filter state for one channel
 */
class BiquadFilterChannel {
  private x1 = 0; // Input delayed by 1 sample
  private x2 = 0; // Input delayed by 2 samples
  private y1 = 0; // Output delayed by 1 sample
  private y2 = 0; // Output delayed by 2 samples

  /**
   * Process one sample through the biquad filter
   * Direct Form I implementation
   */
  process(input: number, coeffs: BiquadCoefficients): number {
    const output =
      coeffs.b0 * input +
      coeffs.b1 * this.x1 +
      coeffs.b2 * this.x2 -
      coeffs.a1 * this.y1 -
      coeffs.a2 * this.y2;

    // Update delay line
    this.x2 = this.x1;
    this.x1 = input;
    this.y2 = this.y1;
    this.y1 = output;

    return output;
  }

  /**
   * Reset filter state (e.g., when changing tracks)
   */
  reset(): void {
    this.x1 = 0;
    this.x2 = 0;
    this.y1 = 0;
    this.y2 = 0;
  }
}

/**
 * Stereo biquad filter
 */
export class BiquadFilter {
  private leftChannel = new BiquadFilterChannel();
  private rightChannel = new BiquadFilterChannel();

  /**
   * Process stereo interleaved buffer in-place
   * @param buffer Stereo interleaved Float32Array [L, R, L, R, ...]
   * @param frames Number of stereo frames
   * @param coeffs Biquad coefficients
   */
  processInterleaved(
    buffer: Float32Array,
    frames: number,
    coeffs: BiquadCoefficients
  ): void {
    for (let i = 0; i < frames; i++) {
      const leftIndex = i * 2;
      const rightIndex = i * 2 + 1;

      buffer[leftIndex] = this.leftChannel.process(buffer[leftIndex], coeffs);
      buffer[rightIndex] = this.rightChannel.process(
        buffer[rightIndex],
        coeffs
      );
    }
  }

  /**
   * Reset filter state
   */
  reset(): void {
    this.leftChannel.reset();
    this.rightChannel.reset();
  }
}

/**
 * Calculate 2nd-order Butterworth lowpass filter coefficients
 * @param fc Cutoff frequency (Hz)
 * @param sampleRate Sample rate (Hz)
 * @returns Biquad coefficients
 */
export function calculateButterworthLowpass(
  fc: number,
  sampleRate: number
): BiquadCoefficients {
  const Q = 0.7071067811865476; // 1/sqrt(2) for Butterworth

  const w0 = (2 * Math.PI * fc) / sampleRate;
  const cosW0 = Math.cos(w0);
  const sinW0 = Math.sin(w0);
  const alpha = sinW0 / (2 * Q);

  const a0 = 1 + alpha;
  const b0 = (1 - cosW0) / 2 / a0;
  const b1 = (1 - cosW0) / a0;
  const b2 = (1 - cosW0) / 2 / a0;
  const a1 = (-2 * cosW0) / a0;
  const a2 = (1 - alpha) / a0;

  return { b0, b1, b2, a1, a2 };
}

/**
 * Calculate 2nd-order Butterworth highpass filter coefficients
 * @param fc Cutoff frequency (Hz)
 * @param sampleRate Sample rate (Hz)
 * @returns Biquad coefficients
 */
export function calculateButterworthHighpass(
  fc: number,
  sampleRate: number
): BiquadCoefficients {
  const Q = 0.7071067811865476; // 1/sqrt(2) for Butterworth

  const w0 = (2 * Math.PI * fc) / sampleRate;
  const cosW0 = Math.cos(w0);
  const sinW0 = Math.sin(w0);
  const alpha = sinW0 / (2 * Q);

  const a0 = 1 + alpha;
  const b0 = (1 + cosW0) / 2 / a0;
  const b1 = -(1 + cosW0) / a0;
  const b2 = (1 + cosW0) / 2 / a0;
  const a1 = (-2 * cosW0) / a0;
  const a2 = (1 - alpha) / a0;

  return { b0, b1, b2, a1, a2 };
}

/**
 * Calculate bandpass filter coefficients (constant peak gain)
 * @param fc Center frequency (Hz)
 * @param Q Q factor (bandwidth)
 * @param sampleRate Sample rate (Hz)
 * @returns Biquad coefficients
 */
export function calculateBandpass(
  fc: number,
  Q: number,
  sampleRate: number
): BiquadCoefficients {
  const w0 = (2 * Math.PI * fc) / sampleRate;
  const cosW0 = Math.cos(w0);
  const sinW0 = Math.sin(w0);
  const alpha = sinW0 / (2 * Q);

  const a0 = 1 + alpha;
  const b0 = alpha / a0;
  const b1 = 0;
  const b2 = -alpha / a0;
  const a1 = (-2 * cosW0) / a0;
  const a2 = (1 - alpha) / a0;

  return { b0, b1, b2, a1, a2 };
}
