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
    // Use first 30 seconds max for analysis
    const maxSamples = Math.min(pcmData.length, sampleRate * 30);
    const data = maxSamples < pcmData.length ? pcmData.subarray(0, maxSamples) : pcmData;

    // Calculate onset strength envelope
    const onsets = this.detectOnsets(data, sampleRate);
    
    // Find tempo using autocorrelation
    const bpm = this.findTempo(onsets, sampleRate);
    
    return bpm;
  }

  /**
   * Detect onsets using energy-based approach
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

    // Normalize
    let maxOnset = 0;
    for (let i = 0; i < onsetStrength.length; i++) {
      if (onsetStrength[i] > maxOnset) maxOnset = onsetStrength[i];
    }
    
    if (maxOnset > 0) {
      for (let i = 0; i < onsetStrength.length; i++) {
        onsetStrength[i] /= maxOnset;
      }
    }

    return onsetStrength;
  }

  /**
   * Find tempo using autocorrelation on onset envelope
   */
  private static findTempo(onsets: Float32Array, sampleRate: number): number | null {
    const hopSize = 512;
    const onsetSampleRate = sampleRate / hopSize;

    // BPM range: 60-180 BPM
    const minBPM = 60;
    const maxBPM = 180;

    const minLag = Math.floor((60 / maxBPM) * onsetSampleRate);
    const maxLag = Math.floor((60 / minBPM) * onsetSampleRate);

    let bestLag = minLag;
    let bestCorr = 0;

    // Calculate autocorrelation
    for (let lag = minLag; lag <= maxLag && lag < onsets.length / 2; lag++) {
      let correlation = 0;
      let count = 0;

      for (let i = 0; i < onsets.length - lag; i++) {
        correlation += onsets[i] * onsets[i + lag];
        count++;
      }

      if (count > 0) {
        correlation /= count;
        
        if (correlation > bestCorr) {
          bestCorr = correlation;
          bestLag = lag;
        }
      }
    }

    if (bestCorr < 0.1) {
      return null; // No clear tempo found
    }

    let bpm = 60 / (bestLag / onsetSampleRate);

    // Check for double-tempo error
    if (bpm < 90) {
      bpm *= 2;
    } else if (bpm > 160) {
      bpm /= 2;
    }

    return Math.round(bpm);
  }
}
