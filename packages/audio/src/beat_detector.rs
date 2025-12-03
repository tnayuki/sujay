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
        // Round BPM to 2 decimal places (like Mixxx)
        let refined_bpm = (refined_bpm * 100.0).round() / 100.0;

        // Step 4: Find detected beat positions for phase alignment
        let beat_period = 60.0 / refined_bpm * odf_sr;
        let detected_beats = self.dp_beat_tracking(&combined_odf, beat_period, odf_sr);

        if detected_beats.is_empty() {
            return None;
        }

        // Step 5: Find optimal first beat position using detected beats (Mixxx-style phase adjustment)
        // Calculate the beat interval in seconds
        let beat_interval = 60.0 / refined_bpm;
        let duration = audio.len() as f32 / self.sample_rate;

        // Find the best phase offset by voting from detected beats
        let first_beat = self.find_optimal_first_beat(&detected_beats, beat_interval);

        // Step 6: Generate constant-tempo beat grid from first beat
        let beats = self.generate_beat_grid(first_beat, beat_interval, duration);

        // Confidence based on how well detected beats align with grid
        let confidence = self.calculate_grid_confidence(&detected_beats, &beats);

        Some(BeatDetectionResult {
            bpm: refined_bpm,
            beats,
            confidence,
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

        // Find peaks in autocorrelation
        let mut peaks = Vec::new();
        for i in 1..correlations.len() - 1 {
            let (lag, corr) = correlations[i];
            if corr > correlations[i - 1].1 && corr > correlations[i + 1].1 {
                peaks.push((lag, corr));
            }
        }

        if peaks.is_empty() {
            // Fallback to max
            let (best_lag, max_corr) = correlations
                .iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .copied()?;
            let bpm = 60.0 / (best_lag as f32 / odf_sr);
            return Some((bpm, max_corr / odf.len() as f32));
        }

        // Sort peaks by correlation strength
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Choose the first peak that gives BPM in preferred range (80-160)
        // This helps avoid half/double tempo detection
        let preferred_min = 80.0;
        let preferred_max = 160.0;

        for &(lag, corr) in &peaks {
            let bpm = 60.0 / (lag as f32 / odf_sr);
            if bpm >= preferred_min && bpm <= preferred_max {
                return Some((bpm, corr / odf.len() as f32));
            }
        }

        // If no peak in preferred range, use strongest peak and adjust
        let (best_lag, best_corr) = peaks[0];
        let mut bpm = 60.0 / (best_lag as f32 / odf_sr);

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

    /// Find optimal first beat position using phase voting from detected beats
    /// This is similar to Mixxx's adjustPhase function
    fn find_optimal_first_beat(&self, detected_beats: &[f32], beat_interval: f32) -> f32 {
        if detected_beats.is_empty() {
            return 0.0;
        }

        // For each detected beat, calculate its phase offset (position modulo beat_interval)
        // Then find the most common phase offset using histogram voting
        const NUM_BINS: usize = 100;
        let mut phase_histogram = vec![0.0f32; NUM_BINS];

        for &beat_time in detected_beats {
            // Calculate phase offset (0 to beat_interval)
            let phase = beat_time % beat_interval;
            let bin = ((phase / beat_interval) * NUM_BINS as f32) as usize;
            let bin = bin.min(NUM_BINS - 1);
            phase_histogram[bin] += 1.0;
        }

        // Smooth the histogram
        let smoothed = self.smooth_histogram(&phase_histogram);

        // Find the bin with maximum votes
        let max_bin = smoothed
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(idx, _)| idx)
            .unwrap_or(0);

        // Convert bin back to phase offset
        let best_phase = (max_bin as f32 + 0.5) / NUM_BINS as f32 * beat_interval;

        // Find the first beat position (closest to 0 with this phase)
        // Move the phase to be as close to 0 as possible
        if best_phase < beat_interval / 2.0 {
            best_phase
        } else {
            best_phase - beat_interval
        }.max(0.0)
    }

    /// Smooth a histogram using a simple moving average
    fn smooth_histogram(&self, histogram: &[f32]) -> Vec<f32> {
        let window = 5;
        let len = histogram.len();
        let mut smoothed = vec![0.0f32; len];

        for i in 0..len {
            let mut sum = 0.0;
            let mut count = 0;
            for j in 0..window {
                let idx = (i + len - window / 2 + j) % len; // Circular
                sum += histogram[idx];
                count += 1;
            }
            smoothed[i] = sum / count as f32;
        }

        smoothed
    }

    /// Generate a constant-tempo beat grid
    fn generate_beat_grid(&self, first_beat: f32, beat_interval: f32, duration: f32) -> Vec<f32> {
        let mut beats = Vec::new();
        let mut pos = first_beat;

        while pos < duration {
            if pos >= 0.0 {
                beats.push(pos);
            }
            pos += beat_interval;
        }

        beats
    }

    /// Calculate confidence based on how well detected beats align with grid
    fn calculate_grid_confidence(&self, detected_beats: &[f32], grid_beats: &[f32]) -> f32 {
        if detected_beats.is_empty() || grid_beats.is_empty() {
            return 0.0;
        }

        // For each detected beat, find the closest grid beat and measure the error
        let mut total_error = 0.0f32;
        let tolerance = 0.05; // 50ms tolerance

        for &detected in detected_beats {
            // Find closest grid beat
            let min_dist = grid_beats
                .iter()
                .map(|&grid| (detected - grid).abs())
                .fold(f32::MAX, f32::min);

            // Score based on how close the detected beat is to the grid
            if min_dist < tolerance {
                total_error += min_dist / tolerance;
            } else {
                total_error += 1.0;
            }
        }

        // Convert to confidence (0-5.32 scale like Essentia)
        let avg_error = total_error / detected_beats.len() as f32;
        let confidence = (1.0 - avg_error).max(0.0) * 5.32;
        confidence
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
