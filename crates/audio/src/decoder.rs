//! MP3 / audio decoder using symphonia with BPM detection and structure analysis.

use std::fs::File;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// A section of a track (intro, main, or outro).
pub struct TrackSection {
    pub start: f64,
    pub end: f64,
    pub beats: i32,
}

/// Full track structure analysis.
pub struct TrackStructure {
    pub bpm: f64,
    pub intro: TrackSection,
    pub main: TrackSection,
    pub outro: TrackSection,
    pub hot_cues: Vec<f64>,
    /// Beat timestamps in seconds.
    pub beats: Vec<f64>,
}

/// Result from decoding an audio file.
pub struct DecodeResult {
    /// Interleaved stereo PCM f32 samples.
    pub pcm: Vec<f32>,
    /// Mono-downmixed PCM f32 samples (for waveform display).
    pub mono: Vec<f32>,
    /// Detected BPM (if successful).
    pub bpm: Option<f64>,
    /// Track structure analysis (if BPM detected).
    pub structure: Option<TrackStructure>,
    /// Output sample rate.
    pub sample_rate: u32,
    /// Number of channels (always 2 for stereo output).
    pub channels: u32,
}

/// Decode an audio file and return PCM data with BPM and structure analysis.
pub fn decode_audio(
    mp3_path: String,
    target_sample_rate: u32,
    target_channels: u32,
) -> Result<DecodeResult, String> {
    let file = File::open(&mp3_path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Failed to probe format: {}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track found".to_string())?;

    let track_id = track.id;
    let source_sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let source_channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        match format.next_packet() {
            Ok(packet) => {
                if packet.track_id() != track_id { continue; }
                match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        let spec = *audio_buf.spec();
                        let duration = audio_buf.capacity() as u64;
                        let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
                        sample_buf.copy_interleaved_ref(audio_buf);
                        all_samples.extend_from_slice(sample_buf.samples());
                    }
                    Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                    Err(e) => return Err(format!("Decode error: {}", e)),
                }
            }
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Format error: {}", e)),
        }
    }

    if all_samples.is_empty() {
        return Err("No samples decoded".to_string());
    }

    let source_frames = all_samples.len() / source_channels;
    let resample_needed = source_sample_rate != target_sample_rate;
    let target_frames = if resample_needed {
        (source_frames as f64 * target_sample_rate as f64 / source_sample_rate as f64) as usize
    } else {
        source_frames
    };
    let sample_rate_ratio = source_sample_rate as f64 / target_sample_rate as f64;

    let mut pcm  = vec![0f32; target_frames * target_channels as usize];
    let mut mono = vec![0f32; target_frames];

    for frame in 0..target_frames {
        let src_index = if resample_needed {
            ((frame as f64 * sample_rate_ratio) as usize).min(source_frames - 1)
        } else {
            frame
        };
        let mut mono_accum = 0f32;
        for ch in 0..target_channels as usize {
            let src_ch = ch.min(source_channels - 1);
            let sample = all_samples[src_index * source_channels + src_ch].clamp(-1.0, 1.0);
            pcm[frame * target_channels as usize + ch] = sample;
            mono_accum += sample;
        }
        mono[frame] = mono_accum / target_channels as f32;
    }

    let bpm = detect_bpm(&mono, target_sample_rate);
    let structure = bpm.map(|detected_bpm| detect_structure(&mono, target_sample_rate, detected_bpm));

    Ok(DecodeResult { pcm, mono, bpm, structure, sample_rate: target_sample_rate, channels: target_channels })
}

// ── BPM Detection ─────────────────────────────────────────────────────────────

fn detect_bpm(mono: &[f32], sample_rate: u32) -> Option<f64> {
    let onsets = detect_onsets(mono);
    find_tempo(&onsets, sample_rate)
}

fn detect_onsets(data: &[f32]) -> Vec<f32> {
    const HOP_SIZE: usize = 512;
    const FRAME_SIZE: usize = 2048;
    if data.len() < FRAME_SIZE { return Vec::new(); }
    let num_frames = (data.len() - FRAME_SIZE) / HOP_SIZE;
    let mut onset_strength = vec![0f32; num_frames];
    let mut prev_energy = 0f32;
    for i in 0..num_frames {
        let start = i * HOP_SIZE;
        let energy: f32 = (data[start..start + FRAME_SIZE].iter().map(|s| s * s).sum::<f32>() / FRAME_SIZE as f32).sqrt();
        let flux = (energy - prev_energy).max(0.0);
        onset_strength[i] = flux;
        prev_energy = energy;
    }
    let window_size = 3i32;
    let mut smoothed = vec![0f32; num_frames];
    for i in 0..num_frames {
        let mut sum = 0f32;
        let mut count = 0;
        for j in -window_size..=window_size {
            let idx = i as i32 + j;
            if idx >= 0 && (idx as usize) < num_frames { sum += onset_strength[idx as usize]; count += 1; }
        }
        smoothed[i] = sum / count as f32;
    }
    let max_onset = smoothed.iter().cloned().fold(0f32, f32::max);
    if max_onset > 0.0 { for s in &mut smoothed { *s /= max_onset; } }
    smoothed
}

fn find_tempo(onsets: &[f32], sample_rate: u32) -> Option<f64> {
    if onsets.is_empty() { return None; }
    const HOP_SIZE: usize = 512;
    let onset_sr = sample_rate as f64 / HOP_SIZE as f64;
    let min_lag = ((60.0 / 200.0) * onset_sr) as usize;
    let max_lag = ((60.0 / 60.0)  * onset_sr) as usize;
    if max_lag >= onsets.len() / 2 { return None; }
    let mut correlations = vec![0f32; max_lag - min_lag + 1];
    for lag in min_lag..=max_lag {
        let count = onsets.len() - lag;
        let correlation: f32 = (0..count).map(|i| onsets[i] * onsets[i + lag]).sum::<f32>() / count as f32;
        correlations[lag - min_lag] = correlation;
    }
    // Parabolic interpolation: given peak at index i, refine to fractional lag.
    let parabolic_lag = |i: usize| -> f64 {
        if i == 0 || i + 1 >= correlations.len() { return (i + min_lag) as f64; }
        let cm1 = correlations[i - 1] as f64;
        let c0  = correlations[i]     as f64;
        let cp1 = correlations[i + 1] as f64;
        let denom = cm1 - 2.0 * c0 + cp1;
        if denom.abs() < 1e-10 { (i + min_lag) as f64 }
        else { (i + min_lag) as f64 + 0.5 * (cm1 - cp1) / denom }
    };
    let mut peaks: Vec<(f64, f32)> = Vec::new(); // (fractional_lag, corr)
    for i in 2..correlations.len().saturating_sub(2) {
        if correlations[i] > correlations[i-1] && correlations[i] > correlations[i+1]
            && correlations[i] > correlations[i-2] && correlations[i] > correlations[i+2]
        {
            peaks.push((parabolic_lag(i), correlations[i]));
        }
    }
    if peaks.is_empty() {
        let (best_idx, &best_corr) = correlations.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap())?;
        if best_corr > 0.0 { return Some(refine_bpm(60.0 / (parabolic_lag(best_idx) / onset_sr))); }
        return None;
    }
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let mut bpm = 60.0 / (peaks[0].0 / onset_sr);
    for i in 1..peaks.len().min(3) {
        let other_bpm = 60.0 / (peaks[i].0 / onset_sr);
        let ratio = bpm / other_bpm;
        if ((ratio - 2.0).abs() < 0.1 || (ratio - 0.5).abs() < 0.1) && peaks[i].1 > peaks[0].1 * 0.8 {
            if other_bpm >= 100.0 && other_bpm <= 140.0 && (bpm < 100.0 || bpm > 140.0) {
                bpm = other_bpm; break;
            }
        }
    }
    Some(refine_bpm(bpm))
}

fn refine_bpm(mut bpm: f64) -> f64 {
    if bpm < 80.0 { bpm *= 2.0; } else if bpm > 170.0 { bpm /= 2.0; }
    // Do NOT round — fractional BPM precision prevents beat grid drift.
    bpm
}

// ── Track Structure ───────────────────────────────────────────────────────────

fn detect_structure(mono: &[f32], sample_rate: u32, bpm: f64) -> TrackStructure {
    let duration = mono.len() as f64 / sample_rate as f64;
    let beat_duration = 60.0 / bpm;
    let energy_envelope = calculate_energy_envelope(mono);
    let (intro_end, outro_start) = detect_section_boundaries(&energy_envelope, sample_rate, bpm, duration);
    let intro_beats  = (intro_end / beat_duration).round() as i32;
    let outro_beats  = ((duration - outro_start) / beat_duration).round() as i32;
    let main_beats   = ((outro_start - intro_end) / beat_duration).round() as i32;
    let mut hot_cues = vec![0.0, intro_end, outro_start];
    if duration > 120.0 { hot_cues.push((intro_end + outro_start) / 2.0); }
    hot_cues.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Use BeatDetector directly (no NAPI)
    let beats = {
        let mut detector = crate::beat_detector::BeatDetector::new(sample_rate as f32);
        detector.detect(mono)
            .map(|r| r.beats.iter().map(|&b| b as f64).collect::<Vec<f64>>())
            .unwrap_or_default()
    };
    TrackStructure {
        bpm,
        intro: TrackSection { start: 0.0, end: intro_end, beats: intro_beats },
        main:  TrackSection { start: intro_end, end: outro_start, beats: main_beats },
        outro: TrackSection { start: outro_start, end: duration, beats: outro_beats },
        hot_cues,
        beats,
    }
}

fn calculate_energy_envelope(mono: &[f32]) -> Vec<f32> {
    const HOP_SIZE: usize = 2048;
    const FRAME_SIZE: usize = 4096;
    if mono.len() < FRAME_SIZE { return Vec::new(); }
    let num_frames = (mono.len() - FRAME_SIZE) / HOP_SIZE;
    let mut energy = vec![0f32; num_frames];
    for i in 0..num_frames {
        let start = i * HOP_SIZE;
        energy[i] = (mono[start..start + FRAME_SIZE].iter().map(|s| s * s).sum::<f32>() / FRAME_SIZE as f32).sqrt();
    }
    let window_size = 5i32;
    let mut smoothed = vec![0f32; num_frames];
    for i in 0..num_frames {
        let mut sum = 0f32; let mut count = 0;
        for j in -window_size..=window_size {
            let idx = i as i32 + j;
            if idx >= 0 && (idx as usize) < num_frames { sum += energy[idx as usize]; count += 1; }
        }
        smoothed[i] = sum / count as f32;
    }
    smoothed
}

fn detect_section_boundaries(energy_envelope: &[f32], sample_rate: u32, bpm: f64, duration: f64) -> (f64, f64) {
    const HOP_SIZE: usize = 2048;
    let beat_duration = 60.0 / bpm;
    let default_intro_end  = 16.0 * beat_duration;
    let default_outro_start = duration - 16.0 * beat_duration;
    if energy_envelope.is_empty() {
        return (default_intro_end.max(0.0), default_outro_start.max(default_intro_end));
    }
    let mean_energy: f32 = energy_envelope.iter().sum::<f32>() / energy_envelope.len() as f32;
    let intro_search_end = (((32.0 * beat_duration * sample_rate as f64) / HOP_SIZE as f64) as usize).min(energy_envelope.len());
    let mut intro_end = default_intro_end;
    for i in 10..intro_search_end {
        let current = energy_envelope[i];
        let previous = energy_envelope[i.saturating_sub(5)];
        if current > previous * 1.5 && current > mean_energy * 0.8 {
            intro_end = (((i * HOP_SIZE) as f64 / sample_rate as f64) / beat_duration).round() * beat_duration;
            break;
        }
    }
    let outro_search_start = energy_envelope.len().saturating_sub(((32.0 * beat_duration * sample_rate as f64) / HOP_SIZE as f64) as usize);
    let mut outro_start = default_outro_start;
    for i in (outro_search_start..energy_envelope.len().saturating_sub(10)).rev() {
        let current = energy_envelope[i];
        let next = energy_envelope[(i + 5).min(energy_envelope.len() - 1)];
        if next < current * 0.7 && next < mean_energy * 0.6 {
            outro_start = (((i * HOP_SIZE) as f64 / sample_rate as f64) / beat_duration).round() * beat_duration;
            break;
        }
    }
    let min_section = 8.0 * beat_duration;
    if outro_start - intro_end < min_section {
        intro_end = default_intro_end;
        outro_start = default_outro_start;
    }
    (intro_end.max(0.0), outro_start.max(intro_end + min_section))
}
