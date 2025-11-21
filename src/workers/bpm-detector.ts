/**
 * BPM Detection using onset detection and autocorrelation
 * Optimized for minimal memory usage
 */

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
    const onsets = this.detectOnsets(data, sampleRate);
    
    // Find tempo using improved autocorrelation with multiple candidates
    const bpm = this.findTempo(onsets, sampleRate);
    
    return bpm;
  }

  /**
   * Detect onsets using energy-based approach with smoothing
   * Returns a decimated onset strength envelope
   */
  private static detectOnsets(data: Float32Array, sampleRate: number): Float32Array {
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
      let bpm = 60 / (lag / onsetSampleRate);
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
