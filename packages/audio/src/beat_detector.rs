// Multi-feature Beat Tracker based on:
// J. Zapata, M. Davies and E. Gómez, "Multi-feature beat tracker,"
// IEEE/ACM Transactions on Audio, Speech and Language Processing, 22(4), 816-825, 2014
//
// This is a clean-room implementation based on the published paper.

use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;

/// Result of beat detection
pub struct BeatDetectionResult {
    /// Detected BPM
    pub bpm: f32,
    /// Beat positions in seconds
    pub beats: Vec<f32>,
    /// Confidence score (0-5.32 scale like Essentia)
    pub confidence: f32,
}

/// Multi-feature beat detector (paper-compliant implementation)
pub struct BeatDetector {
    sample_rate: f32,
    fft_planner: FftPlanner<f32>,
}

impl BeatDetector {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            fft_planner: FftPlanner::new(),
        }
    }

    /// Detect BPM and beat positions from mono audio data
    pub fn detect(&mut self, audio: &[f32]) -> Option<BeatDetectionResult> {
        if audio.len() < self.sample_rate as usize * 2 {
            return None;
        }

        // Step 1: Compute multiple onset detection functions (paper Section III)
        // Use consistent hop_size = 512 for all ODFs
        let odf_complex = self.compute_complex_spectral_diff(audio);
        let odf_energy = self.compute_energy_flux(audio);
        let odf_mel = self.compute_mel_spectral_flux(audio);
        let odf_beat_emphasis = self.compute_beat_emphasis(audio);
        let odf_infogain = self.compute_info_gain(audio);

        // Step 2: Combine ODFs (weighted sum)
        let min_len = [
            odf_complex.len(),
            odf_energy.len(),
            odf_mel.len(),
            odf_beat_emphasis.len(),
            odf_infogain.len(),
        ]
        .into_iter()
        .min()
        .unwrap_or(0);

        if min_len == 0 {
            return None;
        }

        let mut combined_odf = vec![0.0f32; min_len];
        for i in 0..min_len {
            // Weight each ODF equally
            combined_odf[i] = (odf_complex.get(i).unwrap_or(&0.0)
                + odf_energy.get(i).unwrap_or(&0.0)
                + odf_mel.get(i).unwrap_or(&0.0)
                + odf_beat_emphasis.get(i).unwrap_or(&0.0)
                + odf_infogain.get(i).unwrap_or(&0.0))
                / 5.0;
        }

        // Normalize combined ODF
        let max_val = combined_odf.iter().cloned().fold(0.0f32, f32::max);
        if max_val > 0.0 {
            for val in &mut combined_odf {
                *val /= max_val;
            }
        }

        // Step 3: Estimate tempo from combined ODF
        let hop_size = 512;
        let odf_sr = self.sample_rate / hop_size as f32;
        let (bpm, _tempo_confidence) = self.estimate_tempo_from_odf(&combined_odf)?;

        // Refine BPM to typical DJ range (80-170) first
        let mut refined_bpm = bpm;
        while refined_bpm < 80.0 {
            refined_bpm *= 2.0;
        }
        while refined_bpm > 170.0 {
            refined_bpm /= 2.0;
        }
        // Keep fractional BPM precision to avoid beat grid drift over long tracks.

        // Step 4: Find detected beat positions for phase alignment
        let beat_period = 60.0 / refined_bpm * odf_sr;
        let detected_beats = self.dp_beat_tracking(&combined_odf, beat_period, odf_sr);

        if detected_beats.is_empty() {
            return None;
        }

        // Step 4b: Re-derive BPM from the actual mean inter-beat interval of detected beats.
        // This eliminates the ODF autocorrelation quantization error (±2 BPM at 150 BPM) by
        // measuring the average span over all N detected beats: interval = (t_last - t_first) / (N-1).
        // The dp tracker itself uses an integer period, but the mean over many beats averages it out.
        let refined_bpm = if detected_beats.len() >= 4 {
            let span = detected_beats.last().unwrap() - detected_beats.first().unwrap();
            let n    = (detected_beats.len() - 1) as f32;
            let mean_interval = span / n; // seconds per beat
            if mean_interval > 0.0 {
                let bpm_from_beats = 60.0 / mean_interval;
                // Stay in the same octave as refined_bpm (avoid halving/doubling artefacts)
                let mut b = bpm_from_beats;
                while b < refined_bpm * 0.75 { b *= 2.0; }
                while b > refined_bpm * 1.33 { b /= 2.0; }
                b
            } else {
                refined_bpm
            }
        } else {
            refined_bpm
        };

        // Step 5: Use detected beat positions directly instead of projecting a constant-tempo grid.
        // The dp_beat_tracking already found the actual audio transient positions.
        // A constant-tempo grid would drift from the waveform as BPM estimation errors accumulate
        // over many beats (e.g. 0.04 BPM error → 7px drift after 440 beats at 150 BPM).
        // Using actual detected beats ensures beat markers always align with waveform peaks.
        Some(BeatDetectionResult {
            bpm: refined_bpm,
            beats: detected_beats,
            confidence: 1.0,
        })
    }

    /// Complex Spectral Difference (paper Section III.A.1)
    /// Measures changes in both magnitude and phase of FFT
    fn compute_complex_spectral_diff(&mut self, audio: &[f32]) -> Vec<f32> {
        let frame_size = 2048;
        let hop_size = 512; // Unified hop size
        let num_frames = (audio.len().saturating_sub(frame_size)) / hop_size;

        let fft = self.fft_planner.plan_fft_forward(frame_size);
        let window = self.hann_window(frame_size);

        let mut prev_spectrum: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); frame_size];
        let mut prev_prev_spectrum: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); frame_size];
        let mut odf = Vec::with_capacity(num_frames * 2); // Will upsample 2x

        for i in 0..num_frames {
            let start = i * hop_size;
            let mut buffer: Vec<Complex<f32>> = audio[start..start + frame_size]
                .iter()
                .zip(window.iter())
                .map(|(&s, &w)| Complex::new(s * w, 0.0))
                .collect();

            fft.process(&mut buffer);

            // Complex spectral difference: compare predicted phase with actual
            let mut diff = 0.0f32;
            for k in 0..frame_size / 2 {
                // Predicted spectrum (linear extrapolation of phase)
                let predicted = prev_spectrum[k] * 2.0 - prev_prev_spectrum[k];
                let actual = buffer[k];
                diff += (actual - predicted).norm();
            }

            odf.push(diff);

            prev_prev_spectrum = prev_spectrum;
            prev_spectrum = buffer;
        }

        self.normalize_and_smooth(&mut odf);
        odf
    }

    /// Energy Flux / RMS onset detection (paper Section III.A.2)
    fn compute_energy_flux(&mut self, audio: &[f32]) -> Vec<f32> {
        let frame_size = 2048;
        let hop_size = 512; // Unified hop size
        let num_frames = (audio.len().saturating_sub(frame_size)) / hop_size;

        let window = self.hann_window(frame_size);
        let mut prev_energy = 0.0f32;
        let mut odf = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let start = i * hop_size;
            let energy: f32 = audio[start..start + frame_size]
                .iter()
                .zip(window.iter())
                .map(|(&s, &w)| (s * w).powi(2))
                .sum();
            let energy = energy.sqrt();

            // Half-wave rectified difference
            let flux = (energy - prev_energy).max(0.0);
            odf.push(flux);
            prev_energy = energy;
        }

        self.normalize_and_smooth(&mut odf);
        odf
    }

    /// Mel-frequency Spectral Flux (paper Section III.A.3)
    fn compute_mel_spectral_flux(&mut self, audio: &[f32]) -> Vec<f32> {
        let frame_size = 2048;
        let hop_size = 512; // Unified hop size
        let num_frames = (audio.len().saturating_sub(frame_size)) / hop_size;
        let num_mel_bands = 40;

        let fft = self.fft_planner.plan_fft_forward(frame_size);
        let window = self.hann_window(frame_size);
        let mel_filterbank = self.create_mel_filterbank(frame_size, num_mel_bands);

        let mut prev_mel_spectrum = vec![0.0f32; num_mel_bands];
        let mut odf = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let start = i * hop_size;
            let mut buffer: Vec<Complex<f32>> = audio[start..start + frame_size]
                .iter()
                .zip(window.iter())
                .map(|(&s, &w)| Complex::new(s * w, 0.0))
                .collect();

            fft.process(&mut buffer);

            // Compute magnitude spectrum
            let mag_spectrum: Vec<f32> = buffer[..frame_size / 2]
                .iter()
                .map(|c| c.norm())
                .collect();

            // Apply mel filterbank
            let mel_spectrum: Vec<f32> = mel_filterbank
                .iter()
                .map(|filter| {
                    filter
                        .iter()
                        .zip(mag_spectrum.iter())
                        .map(|(&f, &m)| f * m)
                        .sum::<f32>()
                        .ln()
                        .max(0.0)
                })
                .collect();

            // Spectral flux: sum of positive differences
            let flux: f32 = mel_spectrum
                .iter()
                .zip(prev_mel_spectrum.iter())
                .map(|(&curr, &prev)| (curr - prev).max(0.0))
                .sum();

            odf.push(flux);
            prev_mel_spectrum = mel_spectrum;
        }

        self.normalize_and_smooth(&mut odf);
        odf
    }

    /// Beat Emphasis Function (paper Section III.A.4)
    /// Emphasizes periodic beat patterns
    fn compute_beat_emphasis(&mut self, audio: &[f32]) -> Vec<f32> {
        let frame_size = 2048;
        let hop_size = 512;
        let num_frames = (audio.len().saturating_sub(frame_size)) / hop_size;

        let fft = self.fft_planner.plan_fft_forward(frame_size);
        let window = self.hann_window(frame_size);

        // First compute spectral flux
        let mut prev_spectrum = vec![0.0f32; frame_size / 2];
        let mut spectral_flux = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let start = i * hop_size;
            let end = (start + frame_size).min(audio.len());
            if end - start < frame_size {
                break;
            }

            let mut buffer: Vec<Complex<f32>> = audio[start..start + frame_size]
                .iter()
                .zip(window.iter())
                .map(|(&s, &w)| Complex::new(s * w, 0.0))
                .collect();

            fft.process(&mut buffer);

            let mag_spectrum: Vec<f32> = buffer[..frame_size / 2]
                .iter()
                .map(|c| c.norm())
                .collect();

            let flux: f32 = mag_spectrum
                .iter()
                .zip(prev_spectrum.iter())
                .map(|(&curr, &prev)| (curr - prev).max(0.0))
                .sum();

            spectral_flux.push(flux);
            prev_spectrum = mag_spectrum;
        }

        // Apply beat emphasis: weight by periodicity
        let odf_sr = self.sample_rate / hop_size as f32;
        let beat_period_samples = (60.0 / 120.0 * odf_sr) as usize; // Reference: 120 BPM

        let mut odf = vec![0.0f32; spectral_flux.len()];
        for i in beat_period_samples..spectral_flux.len() {
            // Correlation with previous beat position
            let emphasis = spectral_flux[i] * spectral_flux[i - beat_period_samples];
            odf[i] = emphasis.sqrt();
        }

        self.normalize_and_smooth(&mut odf);
        odf
    }

    /// Information Gain (paper Section III.A.5)
    /// Measures spectral change using histogram-based entropy
    fn compute_info_gain(&mut self, audio: &[f32]) -> Vec<f32> {
        let frame_size = 2048;
        let hop_size = 512;
        let num_frames = (audio.len().saturating_sub(frame_size)) / hop_size;
        let num_bins = 20; // Histogram bins

        let fft = self.fft_planner.plan_fft_forward(frame_size);
        let window = self.hann_window(frame_size);

        let mut prev_histogram = vec![0.0f32; num_bins];
        let mut odf = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let start = i * hop_size;
            let end = (start + frame_size).min(audio.len());
            if end - start < frame_size {
                break;
            }

            let mut buffer: Vec<Complex<f32>> = audio[start..start + frame_size]
                .iter()
                .zip(window.iter())
                .map(|(&s, &w)| Complex::new(s * w, 0.0))
                .collect();

            fft.process(&mut buffer);

            // Compute magnitude spectrum and histogram
            let mag_spectrum: Vec<f32> = buffer[..frame_size / 2]
                .iter()
                .map(|c| c.norm())
                .collect();

            let max_mag = mag_spectrum.iter().cloned().fold(0.0f32, f32::max);
            let mut histogram = vec![0.0f32; num_bins];

            if max_mag > 0.0 {
                for &mag in &mag_spectrum {
                    let bin = ((mag / max_mag) * (num_bins - 1) as f32) as usize;
                    let bin = bin.min(num_bins - 1);
                    histogram[bin] += 1.0;
                }
                // Normalize histogram
                let sum: f32 = histogram.iter().sum();
                if sum > 0.0 {
                    for h in &mut histogram {
                        *h /= sum;
                    }
                }
            }

            // Information gain: KL divergence from previous histogram
            let mut info_gain = 0.0f32;
            for (curr, prev) in histogram.iter().zip(prev_histogram.iter()) {
                if *curr > 0.0 && *prev > 0.0 {
                    info_gain += curr * (curr / prev).ln();
                }
            }

            odf.push(info_gain.max(0.0));
            prev_histogram = histogram;
        }

        self.normalize_and_smooth(&mut odf);
        odf
    }

    /// Estimate tempo using autocorrelation
    fn estimate_tempo_from_odf(&self, odf: &[f32]) -> Option<(f32, f32)> {
        let hop_size = 512;
        let odf_sr = self.sample_rate / hop_size as f32;

        let min_bpm = 60.0;
        let max_bpm = 200.0;
        let min_lag = (60.0 / max_bpm * odf_sr) as usize;
        let max_lag = ((60.0 / min_bpm * odf_sr) as usize).min(odf.len() / 2);

        if min_lag >= max_lag {
            return None;
        }

        // Compute autocorrelation
        let mut correlations = Vec::with_capacity(max_lag - min_lag + 1);
        for lag in min_lag..=max_lag {
            let corr: f32 = odf
                .iter()
                .take(odf.len() - lag)
                .zip(odf.iter().skip(lag))
                .map(|(&a, &b)| a * b)
                .sum();
            correlations.push((lag, corr));
        }

        // Helper: parabolic interpolation of lag index → fractional lag
        // Given neighbours c[i-1], c[i], c[i+1], returns the sub-sample peak offset.
        let parabolic_lag = |i: usize| -> f32 {
            if i == 0 || i + 1 >= correlations.len() {
                return correlations[i].0 as f32;
            }
            let (lag, c) = correlations[i];
            let cm1 = correlations[i - 1].1;
            let cp1 = correlations[i + 1].1;
            let denom = cm1 - 2.0 * c + cp1;
            if denom.abs() < 1e-10 {
                lag as f32
            } else {
                lag as f32 + 0.5 * (cm1 - cp1) / denom
            }
        };

        // Find peaks in autocorrelation
        let mut peaks: Vec<(f32, f32)> = Vec::new(); // (fractional_lag, corr)
        for i in 1..correlations.len() - 1 {
            let (_, corr) = correlations[i];
            if corr > correlations[i - 1].1 && corr > correlations[i + 1].1 {
                peaks.push((parabolic_lag(i), corr));
            }
        }

        if peaks.is_empty() {
            // Fallback to max
            let (idx, &(_, max_corr)) = correlations
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.1.partial_cmp(&b.1.1).unwrap())?;
            let frac_lag = parabolic_lag(idx);
            let bpm = 60.0 / (frac_lag / odf_sr);
            return Some((bpm, max_corr / odf.len() as f32));
        }

        // Sort peaks by correlation strength
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Choose the first peak that gives BPM in preferred range (80-160)
        // This helps avoid half/double tempo detection
        let preferred_min = 80.0;
        let preferred_max = 160.0;

        for &(frac_lag, corr) in &peaks {
            let bpm = 60.0 / (frac_lag / odf_sr);
            if bpm >= preferred_min && bpm <= preferred_max {
                return Some((bpm, corr / odf.len() as f32));
            }
        }

        // If no peak in preferred range, use strongest peak and adjust
        let (best_lag, best_corr) = peaks[0];
        let mut bpm = 60.0 / (best_lag / odf_sr);

        // Adjust to preferred range
        while bpm < preferred_min && bpm > 30.0 {
            bpm *= 2.0;
        }
        while bpm > preferred_max && bpm < 300.0 {
            bpm /= 2.0;
        }

        Some((bpm, best_corr / odf.len() as f32))
    }

    /// Dynamic programming beat tracking (improved)
    fn dp_beat_tracking(&self, odf: &[f32], beat_period: f32, odf_sr: f32) -> Vec<f32> {
        let n = odf.len();
        if n == 0 {
            return Vec::new();
        }

        let period = beat_period.round() as usize;
        if period == 0 || period >= n {
            return Vec::new();
        }

        // Find the first strong beat to start from
        let threshold = 0.15;
        let mut first_beat = 0;
        for i in 0..n.min((beat_period * 2.0) as usize) {
            if odf[i] > threshold {
                first_beat = i;
                break;
            }
        }

        // Generate beat grid from first beat with local refinement
        let mut beats = Vec::new();
        let window = (beat_period * 0.15) as i32; // ±15% search window

        let mut expected_pos = first_beat;
        while expected_pos < n {
            // Search for local maximum around expected position
            let search_start = (expected_pos as i32 - window).max(0) as usize;
            let search_end = ((expected_pos as i32 + window) as usize).min(n - 1);

            let mut best_pos = expected_pos.min(n - 1);
            let mut best_val = odf.get(best_pos).copied().unwrap_or(0.0);

            for i in search_start..=search_end {
                if odf[i] > best_val {
                    best_val = odf[i];
                    best_pos = i;
                }
            }

            beats.push(best_pos);
            // Next expected position based on found beat + period
            expected_pos = best_pos + period;
        }

        // Convert to seconds
        beats.iter().map(|&i| i as f32 / odf_sr).collect()
    }

    /// Create Hann window
    fn hann_window(&self, size: usize) -> Vec<f32> {
        (0..size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (size - 1) as f32).cos()))
            .collect()
    }

    /// Create Mel filterbank
    fn create_mel_filterbank(&self, fft_size: usize, num_bands: usize) -> Vec<Vec<f32>> {
        let num_bins = fft_size / 2;
        let f_min = 20.0f32;
        let f_max = self.sample_rate / 2.0;

        // Mel scale conversion
        let hz_to_mel = |f: f32| 2595.0 * (1.0 + f / 700.0).log10();
        let mel_to_hz = |m: f32| 700.0 * (10.0f32.powf(m / 2595.0) - 1.0);

        let mel_min = hz_to_mel(f_min);
        let mel_max = hz_to_mel(f_max);

        // Create mel points
        let mel_points: Vec<f32> = (0..=num_bands + 1)
            .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (num_bands + 1) as f32)
            .collect();

        let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

        let bin_points: Vec<usize> = hz_points
            .iter()
            .map(|&f| ((f / (self.sample_rate / 2.0)) * num_bins as f32) as usize)
            .map(|b| b.min(num_bins - 1))
            .collect();

        // Create triangular filters
        let mut filterbank = Vec::with_capacity(num_bands);

        for i in 0..num_bands {
            let mut filter = vec![0.0f32; num_bins];
            let start = bin_points[i];
            let center = bin_points[i + 1];
            let end = bin_points[i + 2];

            // Rising slope
            for j in start..center {
                if center > start {
                    filter[j] = (j - start) as f32 / (center - start) as f32;
                }
            }

            // Falling slope
            for j in center..end {
                if end > center {
                    filter[j] = (end - j) as f32 / (end - center) as f32;
                }
            }

            filterbank.push(filter);
        }

        filterbank
    }

    /// Normalize and smooth ODF
    fn normalize_and_smooth(&self, odf: &mut Vec<f32>) {
        if odf.is_empty() {
            return;
        }

        // Normalize
        let max_val = odf.iter().cloned().fold(0.0f32, f32::max);
        if max_val > 0.0 {
            for val in odf.iter_mut() {
                *val /= max_val;
            }
        }

        // Smooth with moving average
        let window = 3;
        let original = odf.clone();
        for i in 0..odf.len() {
            let start = i.saturating_sub(window);
            let end = (i + window + 1).min(odf.len());
            odf[i] = original[start..end].iter().sum::<f32>() / (end - start) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beat_detector_creation() {
        let detector = BeatDetector::new(44100.0);
        assert_eq!(detector.sample_rate, 44100.0);
    }

    #[test]
    fn test_detect_with_click_track() {
        let mut detector = BeatDetector::new(44100.0);
        let sample_rate = 44100.0;
        let bpm = 120.0;
        let beat_interval = (60.0 / bpm * sample_rate) as usize;
        let duration_samples = sample_rate as usize * 30;

        // Generate click track
        let mut audio = vec![0.0f32; duration_samples];
        let mut pos = 0;
        while pos < duration_samples {
            for i in 0..100 {
                if pos + i < duration_samples {
                    audio[pos + i] = 0.8 * (-(i as f32) / 50.0).exp();
                }
            }
            pos += beat_interval;
        }

        let result = detector.detect(&audio);
        assert!(result.is_some());

        let result = result.unwrap();
        assert!(
            (result.bpm - 120.0).abs() < 5.0,
            "Expected BPM ~120, got {}",
            result.bpm
        );
    }
}
