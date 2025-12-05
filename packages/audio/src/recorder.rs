use std::fs::File;
use std::io::BufWriter;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use napi::Result;
use vorbis_rs::{VorbisEncoder, VorbisEncoderBuilder};
use std::num::{NonZeroU32, NonZeroU8};
use napi_derive::napi;

#[napi]
pub enum RecordingFormat {
    Wav,
    Ogg,
}

enum RecordingMessage {
    Start { path: String, format: RecordingFormat },
    AudioData(Vec<f32>),
    Stop,
}

trait AudioWriter {
    fn write_samples(&mut self, samples: &[f32]) -> Result<()>;
    fn finalize(self: Box<Self>) -> Result<()>;
}

struct WavWriter {
    writer: hound::WavWriter<BufWriter<File>>,
}

struct OggWriter {
    encoder: VorbisEncoder<BufWriter<File>>,
}

impl OggWriter {
    fn new(path: &str, sample_rate: u32) -> Result<Self> {
        let f = File::create(path)
            .map_err(|e| napi::Error::from_reason(format!("Failed to create OGG file: {}", e)))?;
        let writer = BufWriter::new(f);

        let sampling_frequency = NonZeroU32::new(sample_rate).ok_or_else(|| napi::Error::from_reason("Invalid sample rate"))?;
        let channels = NonZeroU8::new(2).ok_or_else(|| napi::Error::from_reason("Invalid channel count"))?;

        let mut builder = VorbisEncoderBuilder::new_with_serial(sampling_frequency, channels, writer, 0);
        let encoder = builder.build()
            .map_err(|e| napi::Error::from_reason(format!("Failed to create Vorbis encoder: {}", e)))?;
        Ok(Self { encoder })
    }
}

impl WavWriter {
    fn new(path: &str, sample_rate: u32) -> Result<Self> {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(path, spec)
            .map_err(|e| napi::Error::from_reason(format!("Failed to create WAV file: {}", e)))?;
        Ok(Self { writer })
    }
}

impl AudioWriter for WavWriter {
    fn write_samples(&mut self, samples: &[f32]) -> Result<()> {
        for &sample in samples {
            let clamped = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            self.writer.write_sample(clamped)
                .map_err(|e| napi::Error::from_reason(format!("Failed to write WAV sample: {}", e)))?;
        }
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<()> {
        self.writer.finalize()
            .map_err(|e| napi::Error::from_reason(format!("Failed to finalize WAV file: {}", e)))?;
        Ok(())
    }
}

impl AudioWriter for OggWriter {
    fn write_samples(&mut self, samples: &[f32]) -> Result<()> {
        // Interleaved stereo -> planar channels
        let channels = 2usize;
        if samples.len() % channels != 0 { return Err(napi::Error::from_reason("Invalid sample length")); }
        let frames = samples.len() / channels;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for i in 0..frames {
            left.push(samples[i*2]);
            right.push(samples[i*2 + 1]);
        }
        let blocks: [&[f32]; 2] = [&left[..], &right[..]];
        self.encoder.encode_audio_block(&blocks)
            .map_err(|e| napi::Error::from_reason(format!("Vorbis encode error: {}", e)))?;
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<()> {
        // Explicitly consume the encoder to finalize OGG stream
        self.encoder.finish()
            .map_err(|e| napi::Error::from_reason(format!("Vorbis finalize error: {}", e)))?;
        Ok(())
    }
}

pub struct RecordingThread {
    thread: Option<JoinHandle<()>>,
    sender: Option<Sender<RecordingMessage>>,
}

impl RecordingThread {
    pub fn new() -> Self {
        Self {
            thread: None,
            sender: None,
        }
    }

    pub fn start_recording(&mut self, path: String, format: RecordingFormat) -> Result<()> {
        if self.thread.is_some() {
            return Err(napi::Error::from_reason("Recording already in progress"));
        }

        let (sender, receiver) = mpsc::channel();
        self.sender = Some(sender);

        let thread = thread::spawn(move || {
            Self::recording_loop(receiver);
        });
        self.thread = Some(thread);

        // Send start message
        if let Some(ref sender) = self.sender {
            sender.send(RecordingMessage::Start { path, format })
                .map_err(|_| napi::Error::from_reason("Failed to send start message"))?;
        }

        Ok(())
    }

    pub fn send_audio_data(&mut self, data: &[f32]) {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(RecordingMessage::AudioData(data.to_vec()));
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(sender) = self.sender.take() {
            sender.send(RecordingMessage::Stop)
                .map_err(|_| napi::Error::from_reason("Failed to send stop message"))?;
        }

        if let Some(thread) = self.thread.take() {
            thread.join()
                .map_err(|_| napi::Error::from_reason("Recording thread panicked"))?;
        }

        Ok(())
    }

    fn recording_loop(receiver: Receiver<RecordingMessage>) {
        let mut writer: Option<Box<dyn AudioWriter>> = None;
        let sample_rate = 44100; // Should match AudioEngine sample rate

        while let Ok(message) = receiver.recv() {
            match message {
                RecordingMessage::Start { path, format } => {
                    writer = match format {
                            RecordingFormat::Wav => Some(Box::new(WavWriter::new(&path, sample_rate).unwrap())),
                            RecordingFormat::Ogg => Some(Box::new(OggWriter::new(&path, sample_rate).unwrap())),
                    };
                }
                RecordingMessage::AudioData(data) => {
                    if let Some(ref mut w) = writer {
                        let _ = w.write_samples(&data);
                    }
                }
                RecordingMessage::Stop => {
                    if let Some(w) = writer.take() {
                        let _ = w.finalize();
                    }
                    break;
                }
            }
        }
    }
}

impl Drop for RecordingThread {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}