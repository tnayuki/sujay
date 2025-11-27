/**
 * BPM Detection using onset detection and autocorrelation
 * Optimized for minimal memory usage
 */

import type { TrackStructure, TrackSection } from '../types.js';

export class BPMDetector {
  /**
   * Detect BPM from mono Float32Array audio data
   * @param pcmData - Float32Array of mono audio samples (-1.0 to 1.0)
   * @param sampleRate - Sample rate (e.g., 44100)
   * @returns Detected BPM or null if detection fails
   */
  static detect(pcmData: Float32Array, sampleRate: number): number | null {
    // Analyze the entire track for best accuracy
    const data = pcmData;

    // Calculate onset strength envelope
    const onsets = this.detectOnsets(data);
    
    // Find tempo using improved autocorrelation with multiple candidates
    const bpm = this.findTempo(onsets, sampleRate);
    
    return bpm;
  }

  /**
   * Detect track structure (intro/main/outro sections)
   * @param pcmData - Float32Array of mono audio samples (-1.0 to 1.0)
   * @param sampleRate - Sample rate (e.g., 44100)
   * @param bpm - Detected BPM (if null, will be detected automatically)
   * @returns Track structure analysis or null if detection fails
   */
  static detectStructure(
    pcmData: Float32Array,
    sampleRate: number,
    bpm: number | null = null
  ): TrackStructure | null {
    // Detect BPM if not provided
    const detectedBpm = bpm || this.detect(pcmData, sampleRate);
    if (!detectedBpm) {
      return null;
    }

    const duration = pcmData.length / sampleRate;
    const beatDuration = 60 / detectedBpm; // seconds per beat

    // Calculate energy envelope for the entire track
    const energyEnvelope = this.calculateEnergyEnvelope(pcmData, sampleRate);

    // Detect intro/outro boundaries using energy analysis
    const { introEnd, outroStart } = this.detectSectionBoundaries(
      energyEnvelope,
      sampleRate,
      detectedBpm,
      duration
    );

    // Convert to TrackSection format
    const introBeats = Math.round(introEnd / beatDuration);
    const outroBeats = Math.round((duration - outroStart) / beatDuration);

    const intro: TrackSection = {
      start: 0,
      end: introEnd,
      beats: introBeats,
    };

    const main: TrackSection = {
      start: introEnd,
      end: outroStart,
      beats: Math.round((outroStart - introEnd) / beatDuration),
    };

    const outro: TrackSection = {
      start: outroStart,
      end: duration,
      beats: outroBeats,
    };

    // Generate hot cues at important positions
    const hotCues: number[] = [
      0,                    // Start
      introEnd,             // Intro end / main start (drop point)
      outroStart,           // Outro start (mix out point)
    ];

    // Add mid-point cue if track is long enough
    if (duration > 120) {
      hotCues.push((introEnd + outroStart) / 2);
    }

    return {
      bpm: detectedBpm,
      intro,
      main,
      outro,
      hotCues: hotCues.sort((a, b) => a - b),
    };
  }

  /**
   * Calculate energy envelope of the audio
   * Returns decimated RMS energy values
   */
  private static calculateEnergyEnvelope(
    pcmData: Float32Array,
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    sampleRate: number
  ): Float32Array {
    const hopSize = 2048; // ~46ms at 44.1kHz
    const frameSize = 4096;
    const numFrames = Math.floor((pcmData.length - frameSize) / hopSize);

    const energy = new Float32Array(numFrames);

    for (let i = 0; i < numFrames; i++) {
      const start = i * hopSize;
      let sum = 0;

      for (let j = 0; j < frameSize; j++) {
        const sample = pcmData[start + j];
        sum += sample * sample;
      }

      energy[i] = Math.sqrt(sum / frameSize); // RMS
    }

    // Smooth the envelope
    const smoothed = new Float32Array(numFrames);
    const windowSize = 5;

    for (let i = 0; i < numFrames; i++) {
      let sum = 0;
      let count = 0;

      for (let j = -windowSize; j <= windowSize; j++) {
        const idx = i + j;
        if (idx >= 0 && idx < numFrames) {
          sum += energy[idx];
          count++;
        }
      }

      smoothed[i] = sum / count;
    }

    return smoothed;
  }

  /**
   * Detect section boundaries (intro end, outro start) using energy analysis
   */
  private static detectSectionBoundaries(
    energyEnvelope: Float32Array,
    sampleRate: number,
    bpm: number,
    duration: number
  ): { introEnd: number; outroStart: number } {
    const hopSize = 2048;
    const beatDuration = 60 / bpm;

    // Default to 16 beats for intro/outro (typical for techno/house)
    const defaultIntroBeats = 16;
    const defaultOutroBeats = 16;

    const defaultIntroEnd = defaultIntroBeats * beatDuration;
    const defaultOutroStart = duration - defaultOutroBeats * beatDuration;

    // Calculate mean energy for reference
    let totalEnergy = 0;
    for (let i = 0; i < energyEnvelope.length; i++) {
      totalEnergy += energyEnvelope[i];
    }
    const meanEnergy = totalEnergy / energyEnvelope.length;

    // Find intro end: first significant energy increase
    let introEnd = defaultIntroEnd;
    const introSearchEnd = Math.min(
      energyEnvelope.length,
      Math.floor((defaultIntroBeats * 2 * beatDuration * sampleRate) / hopSize)
    );

    for (let i = 10; i < introSearchEnd; i++) {
      const currentEnergy = energyEnvelope[i];
      const previousEnergy = energyEnvelope[i - 5];

      // Look for energy jump > 50% AND above mean
      if (
        currentEnergy > previousEnergy * 1.5 &&
        currentEnergy > meanEnergy * 0.8
      ) {
        introEnd = (i * hopSize) / sampleRate;
        // Round to nearest beat
        introEnd = Math.round(introEnd / beatDuration) * beatDuration;
        break;
      }
    }

    // Find outro start: last significant energy drop
    let outroStart = defaultOutroStart;
    const outroSearchStart = Math.max(
      0,
      energyEnvelope.length -
        Math.floor((defaultOutroBeats * 2 * beatDuration * sampleRate) / hopSize)
    );

    for (let i = energyEnvelope.length - 10; i > outroSearchStart; i--) {
      const currentEnergy = energyEnvelope[i];
      const nextEnergy = energyEnvelope[i + 5];

      // Look for energy drop > 30% AND below mean
      if (
        nextEnergy < currentEnergy * 0.7 &&
        nextEnergy < meanEnergy * 0.6
      ) {
        outroStart = (i * hopSize) / sampleRate;
        // Round to nearest beat
        outroStart = Math.round(outroStart / beatDuration) * beatDuration;
        break;
      }
    }

    // Ensure sections don't overlap and have minimum size
    const minSectionDuration = 8 * beatDuration; // At least 8 beats
    if (outroStart - introEnd < minSectionDuration) {
      // Reset to defaults if detection failed
      introEnd = defaultIntroEnd;
      outroStart = defaultOutroStart;
    }

    return { introEnd, outroStart };
  }

  /**
   * Detect onsets using energy-based approach with smoothing
   * Returns a decimated onset strength envelope
   */
  private static detectOnsets(data: Float32Array): Float32Array {
    const hopSize = 512;
    const frameSize = 2048;
    const numFrames = Math.floor((data.length - frameSize) / hopSize);
    
    const onsetStrength = new Float32Array(numFrames);
    let prevEnergy = 0;

    for (let i = 0; i < numFrames; i++) {
      const start = i * hopSize;
      
      // Calculate frame energy (RMS)
      let energy = 0;
      for (let j = 0; j < frameSize; j++) {
        const sample = data[start + j];
        energy += sample * sample;
      }
      energy = Math.sqrt(energy / frameSize);
      
      // Spectral flux: positive difference from previous frame
      const flux = Math.max(0, energy - prevEnergy);
      onsetStrength[i] = flux;
      prevEnergy = energy;
    }

    // Apply smoothing to reduce noise
    const smoothed = new Float32Array(numFrames);
    const windowSize = 3;
    for (let i = 0; i < numFrames; i++) {
      let sum = 0;
      let count = 0;
      for (let j = -windowSize; j <= windowSize; j++) {
        const idx = i + j;
        if (idx >= 0 && idx < numFrames) {
          sum += onsetStrength[idx];
          count++;
        }
      }
      smoothed[i] = sum / count;
    }

    // Normalize
    let maxOnset = 0;
    for (let i = 0; i < smoothed.length; i++) {
      if (smoothed[i] > maxOnset) maxOnset = smoothed[i];
    }
    
    if (maxOnset > 0) {
      for (let i = 0; i < smoothed.length; i++) {
        smoothed[i] /= maxOnset;
      }
    }

    return smoothed;
  }
  /**
   * Find tempo using autocorrelation on onset envelope with multiple candidate peaks
   */
  private static findTempo(onsets: Float32Array, sampleRate: number): number | null {
    const hopSize = 512;
    const onsetSampleRate = sampleRate / hopSize;

    // Extended BPM range: 60-200 BPM
    const minBPM = 60;
    const maxBPM = 200;

    const minLag = Math.floor((60 / maxBPM) * onsetSampleRate);
    const maxLag = Math.floor((60 / minBPM) * onsetSampleRate);

    // Store correlation values
    const correlations = new Float32Array(maxLag - minLag + 1);

    // Calculate autocorrelation
    for (let lag = minLag; lag <= maxLag && lag < onsets.length / 2; lag++) {
      let correlation = 0;
      let count = 0;

      for (let i = 0; i < onsets.length - lag; i++) {
        correlation += onsets[i] * onsets[i + lag];
        count++;
      }

      if (count > 0) {
        correlations[lag - minLag] = correlation / count;
      }
    }

    // Find multiple peaks in correlation function
    const peaks: Array<{lag: number, corr: number, bpm: number}> = [];
    
    for (let i = 2; i < correlations.length - 2; i++) {
      // Peak detection: local maximum
      if (correlations[i] > correlations[i - 1] && 
          correlations[i] > correlations[i + 1] &&
          correlations[i] > correlations[i - 2] && 
          correlations[i] > correlations[i + 2]) {
        const lag = i + minLag;
        const bpm = 60 / (lag / onsetSampleRate);
        peaks.push({ lag, corr: correlations[i], bpm });
      }
    }

    if (peaks.length === 0) {
      // Fallback to max correlation
      let bestIdx = 0;
      let bestCorr = correlations[0];
      for (let i = 1; i < correlations.length; i++) {
        if (correlations[i] > bestCorr) {
          bestCorr = correlations[i];
          bestIdx = i;
        }
      }
      const lag = bestIdx + minLag;
      const bpm = 60 / (lag / onsetSampleRate);
      return this.refineBPM(bpm);
    }

    // Sort peaks by correlation strength
    peaks.sort((a, b) => b.corr - a.corr);

    // Take the strongest peak and refine it
    let bpm = peaks[0].bpm;
    
    // Consider harmonic relationships (check if double/half tempo makes more sense)
    for (let i = 1; i < Math.min(3, peaks.length); i++) {
      const ratio = peaks[0].bpm / peaks[i].bpm;
      // If there's a strong peak at half/double tempo with similar correlation
      if ((Math.abs(ratio - 2) < 0.1 || Math.abs(ratio - 0.5) < 0.1) &&
          peaks[i].corr > peaks[0].corr * 0.8) {
        // Prefer BPM in the 100-140 range (typical dance music)
        if (peaks[i].bpm >= 100 && peaks[i].bpm <= 140 && 
            (peaks[0].bpm < 100 || peaks[0].bpm > 140)) {
          bpm = peaks[i].bpm;
          break;
        }
      }
    }

    return this.refineBPM(bpm);
  }

  /**
   * Refine BPM to common ranges
   */
  private static refineBPM(bpm: number): number {
    // Adjust for common tempo errors
    if (bpm < 80) {
      bpm *= 2; // Double-time correction
    } else if (bpm > 170) {
      bpm /= 2; // Half-time correction
    }

    return Math.round(bpm);
  }
}
