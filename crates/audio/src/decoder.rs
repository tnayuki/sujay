//! MP3 / audio decoder using symphonia.

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
    /// Detected BPM, if supplied by a higher-level analysis source.
    pub bpm: Option<f64>,
    /// Track structure analysis, if supplied by a higher-level analysis source.
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
    let file = File::open(&mp3_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "Failed to open file: {} (path: {}). On macOS, allow Music folder access for the process (Terminal/VS Code/Sujay).",
                e,
                mp3_path
            )
        } else {
            format!("Failed to open file: {} (path: {})", e, mp3_path)
        }
    })?;
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

    let bpm = None;
    let structure = None;

    Ok(DecodeResult { pcm, mono, bpm, structure, sample_rate: target_sample_rate, channels: target_channels })
}

