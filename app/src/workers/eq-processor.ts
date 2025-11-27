/**
 * 3-Band EQ Processor with Kill Switches
 * Implements DJ-style frequency isolation matching Pioneer DJM mixers
 * Frequency bands:
 * - LOW: Below 250 Hz (lowpass)
 * - MID: 250 Hz to 5 kHz (bandpass)
 * - HIGH: Above 5 kHz (highpass)
 * 
 * Uses overlapping filters for smooth transitions, similar to analog DJ mixers.
 */

import {
  BiquadFilter,
  BiquadCoefficients,
  calculateButterworthLowpass,
  calculateButterworthHighpass,
} from './biquad-filter';

const SAMPLE_RATE = 44100;

// DJ mixer style frequency bands (overlapping for smooth transitions)
const FREQ_LOW = 250;   // Low band cutoff
const FREQ_MID_LOW = 250;  // Mid band low cutoff  
const FREQ_MID_HIGH = 5000; // Mid band high cutoff
const FREQ_HIGH = 5000;  // High band cutoff

/**
 * EQ band identifiers
 */
export type EqBand = 'low' | 'mid' | 'high';

/**
 * EQ cut state (kill switches)
 */
export interface EqCutState {
  low: boolean;
  mid: boolean;
  high: boolean;
}

/**
 * DJ-style 3-band EQ with overlapping filters
 * 
 * Each band uses independent filters for smooth, musical response
 */
export class EqProcessor {
  // Low band: Butterworth LPF at 250Hz
  private lowFilter1 = new BiquadFilter();
  private lowFilter2 = new BiquadFilter();
  private lowCoeffs1: BiquadCoefficients;
  private lowCoeffs2: BiquadCoefficients;

  // Mid band: Bandpass 250Hz to 5kHz
  private midFilterLow1 = new BiquadFilter();
  private midFilterLow2 = new BiquadFilter();
  private midFilterHigh1 = new BiquadFilter();
  private midFilterHigh2 = new BiquadFilter();
  private midCoeffsLow1: BiquadCoefficients;
  private midCoeffsLow2: BiquadCoefficients;
  private midCoeffsHigh1: BiquadCoefficients;
  private midCoeffsHigh2: BiquadCoefficients;

  // High band: Butterworth HPF at 5kHz
  private highFilter1 = new BiquadFilter();
  private highFilter2 = new BiquadFilter();
  private highCoeffs1: BiquadCoefficients;
  private highCoeffs2: BiquadCoefficients;

  // Kill states
  private cutState: EqCutState = {
    low: false,
    mid: false,
    high: false,
  };

  // Temporary buffers for band processing
  private lowBuffer: Float32Array;
  private midBuffer: Float32Array;
  private highBuffer: Float32Array;

  constructor(maxFrames = 2048) {
    // Low band: 2x Butterworth LPF at 250Hz
    this.lowCoeffs1 = calculateButterworthLowpass(FREQ_LOW, SAMPLE_RATE);
    this.lowCoeffs2 = calculateButterworthLowpass(FREQ_LOW, SAMPLE_RATE);

    // Mid band: Bandpass created by HPF (250Hz) + LPF (5kHz)
    this.midCoeffsLow1 = calculateButterworthHighpass(FREQ_MID_LOW, SAMPLE_RATE);
    this.midCoeffsLow2 = calculateButterworthHighpass(FREQ_MID_LOW, SAMPLE_RATE);
    this.midCoeffsHigh1 = calculateButterworthLowpass(FREQ_MID_HIGH, SAMPLE_RATE);
    this.midCoeffsHigh2 = calculateButterworthLowpass(FREQ_MID_HIGH, SAMPLE_RATE);

    // High band: 2x Butterworth HPF at 5kHz
    this.highCoeffs1 = calculateButterworthHighpass(FREQ_HIGH, SAMPLE_RATE);
    this.highCoeffs2 = calculateButterworthHighpass(FREQ_HIGH, SAMPLE_RATE);

    // Allocate temporary buffers (stereo interleaved)
    this.lowBuffer = new Float32Array(maxFrames * 2);
    this.midBuffer = new Float32Array(maxFrames * 2);
    this.highBuffer = new Float32Array(maxFrames * 2);
  }

  /**
   * Set kill state for a specific band
   */
  setCut(band: EqBand, enabled: boolean): void {
    this.cutState[band] = enabled;
  }

  /**
   * Get current cut state
   */
  getCutState(): EqCutState {
    return { ...this.cutState };
  }

  /**
   * Process audio buffer with 3-band EQ and kill switches
   * Uses independent overlapping filters for each band
   * @param buffer Stereo interleaved Float32Array [L, R, L, R, ...]
   * @param frames Number of stereo frames
   */
  process(buffer: Float32Array, frames: number): void {
    const { low, mid, high } = this.cutState;

    // Optimization: bypass EQ if all bands are enabled (no kills active)
    if (!low && !mid && !high) {
      return; // No processing needed
    }

    // Optimization: complete silence if all bands are killed
    if (low && mid && high) {
      buffer.fill(0, 0, frames * 2);
      return;
    }

    const samples = frames * 2; // Stereo interleaved

    // Copy input to all band buffers
    this.lowBuffer.set(buffer.subarray(0, samples));
    this.midBuffer.set(buffer.subarray(0, samples));
    this.highBuffer.set(buffer.subarray(0, samples));

    // Apply filters to each band independently
    // Low: 2x LPF at 250Hz
    this.lowFilter1.processInterleaved(this.lowBuffer, frames, this.lowCoeffs1);
    this.lowFilter2.processInterleaved(this.lowBuffer, frames, this.lowCoeffs2);

    // Mid: HPF at 250Hz then LPF at 5kHz (creates bandpass)
    this.midFilterLow1.processInterleaved(this.midBuffer, frames, this.midCoeffsLow1);
    this.midFilterLow2.processInterleaved(this.midBuffer, frames, this.midCoeffsLow2);
    this.midFilterHigh1.processInterleaved(this.midBuffer, frames, this.midCoeffsHigh1);
    this.midFilterHigh2.processInterleaved(this.midBuffer, frames, this.midCoeffsHigh2);

    // High: 2x HPF at 5kHz
    this.highFilter1.processInterleaved(this.highBuffer, frames, this.highCoeffs1);
    this.highFilter2.processInterleaved(this.highBuffer, frames, this.highCoeffs2);

    // Mix bands with kill switches applied
    for (let i = 0; i < samples; i++) {
      buffer[i] =
        (low ? 0 : this.lowBuffer[i]) +
        (mid ? 0 : this.midBuffer[i]) +
        (high ? 0 : this.highBuffer[i]);
    }
  }

  /**
   * Reset all filter states (e.g., when changing tracks)
   */
  reset(): void {
    this.lowFilter1.reset();
    this.lowFilter2.reset();
    this.midFilterLow1.reset();
    this.midFilterLow2.reset();
    this.midFilterHigh1.reset();
    this.midFilterHigh2.reset();
    this.highFilter1.reset();
    this.highFilter2.reset();
  }
}
