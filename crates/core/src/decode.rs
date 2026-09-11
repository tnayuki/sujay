//! Background decode of a track for a deck: PCM, waveform, beat grid, cues,
//! with the rekordbox override (beat grid and cues fetched per track) applied.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::library::RekordboxTrackOverride;
use crate::state::DeckCueVisualState;

/// Result of a background decode, sent back to the main thread for loading.
pub struct DecodeReady {
  pub deck: u8,
  pub pcm: Vec<f32>,
  pub waveform: Vec<f32>,
  pub waveform_colors: Vec<[u8; 3]>,
  pub bpm: Option<f32>,
  pub title: String,
  /// Beat positions in audio frames.
  pub beats: Vec<f32>,
  /// Intro position in audio frames (if detected).
  pub intro: Option<f32>,
  /// Outro position in audio frames (if detected).
  pub outro: Option<f32>,
  pub cues: Vec<DeckCueVisualState>,
  /// Total mono frames (pcm.len() / 2).
  pub total_frames: f32,
}

pub struct DecodeFailure {
  pub deck: u8,
  pub path: PathBuf,
  pub error: String,
}

/// Everything a decode produces, before it is sent to the main thread.
struct DecodedTrack {
  pcm: Vec<f32>,
  bpm: Option<f32>,
  beats: Vec<f32>,
  intro: Option<f32>,
  outro: Option<f32>,
  waveform: Vec<f32>,
  waveform_colors: Vec<[u8; 3]>,
  cues: Vec<DeckCueVisualState>,
  title: String,
}

pub enum DecodeResult {
  Ready(DecodeReady),
  Failed(DecodeFailure),
}

/// Decode a source file and resolve its metadata (BPM, beats, waveform), applying
/// any rekordbox override.  Runs the decode under `catch_unwind` so a codec
/// panic becomes an `Err`.
fn decode_track_full(
  path: &Path,
  override_meta: Option<RekordboxTrackOverride>,
) -> Result<DecodedTrack, String> {
  let path_str = path.to_string_lossy().to_string();
  let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    sujay_audio::decoder::decode_audio(path_str.clone(), 44100, 2)
  }));
  let result = match res {
    Err(e) => {
      let msg = e
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| e.downcast_ref::<String>().map(|s| s.as_str()))
        .unwrap_or("(unknown panic)");
      return Err(format!("Decode panicked: {msg}"));
    }
    Ok(Err(e)) => return Err(e.to_string()),
    Ok(Ok(result)) => result,
  };

  let sr = result.sample_rate as f32;
  let mut bpm = result.bpm.map(|b| b as f32);
  // 44100 Hz → ~200 Hz (step=220): 5min track ≈ 60k points.
  let step = (result.sample_rate as usize / 200).max(1);
  let mut waveform: Vec<f32> = result
    .mono
    .chunks(step)
    .map(|chunk| chunk.iter().map(|&s| s.abs()).fold(0.0f32, f32::max))
    .collect();
  let mut waveform_colors = Vec::new();
  let mut rekordbox_cues = Vec::new();
  let mut title = path
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_default();
  // Convert beat/intro/outro from seconds → audio frames
  let (mut beats, mut intro, mut outro) = if let Some(ref st) = result.structure {
    let beats = st.beats.iter().map(|&s| s as f32 * sr).collect();
    let intro = Some(st.intro.end as f32 * sr);
    let outro = Some(st.outro.start as f32 * sr);
    (beats, intro, outro)
  } else {
    (vec![], None, None)
  };

  if let Some(meta) = override_meta {
    if !meta.title.is_empty() {
      title = meta.title;
    }
    if meta.bpm.is_some() {
      bpm = meta.bpm;
    }
    if !meta.waveform.is_empty() {
      waveform = meta.waveform;
    }
    if !meta.waveform_colors.is_empty() {
      waveform_colors = meta.waveform_colors;
    }
    if !meta.beats_ms.is_empty() {
      beats = meta.beats_ms.iter().map(|ms| (*ms / 1000.0) * sr).collect();
      intro = None;
      outro = None;
    }
    rekordbox_cues = meta.cues;
  }
  let total_frames = result.pcm.len() / 2;
  let cues = rekordbox_cues_to_visuals(&rekordbox_cues, total_frames, sr);

  Ok(DecodedTrack {
    pcm: result.pcm,
    bpm,
    beats,
    intro,
    outro,
    waveform,
    waveform_colors,
    cues,
    title,
  })
}

/// Decode `path` on a background thread and send the result via `tx`.
pub fn spawn_decode(
  deck: u8,
  path: PathBuf,
  tx: Sender<DecodeResult>,
  mut override_meta: Option<RekordboxTrackOverride>,
  master_db_path: Option<PathBuf>,
  rekordbox_content_id: Option<String>,
) {
  std::thread::spawn(move || {
    eprintln!("[D&D] Decoding {} for deck {}", path.display(), deck);
    if let (Some(meta), Some(master_db_path), Some(content_id)) = (
      override_meta.as_mut(),
      master_db_path.as_deref(),
      rekordbox_content_id.as_deref(),
    ) {
      for attempt in 1..=3 {
        match sujay_library::load_track_beats_from_master_db(master_db_path, content_id) {
          Ok(beats_ms) => {
            eprintln!(
              "[Rekordbox] loaded beat grid for {}: {} markers",
              path.display(),
              beats_ms.len()
            );
            meta.beats_ms = beats_ms;
            break;
          }
          Err(err) if attempt < 3 => {
            eprintln!(
              "[Rekordbox] beat grid read failed for {} (attempt {attempt}/3): {err}",
              path.display()
            );
            std::thread::sleep(Duration::from_millis(100 * attempt));
          }
          Err(err) => eprintln!(
            "[Rekordbox] beat grid unavailable for {} after {attempt} attempts: {err}",
            path.display()
          ),
        }
      }
      match sujay_library::load_track_cues_from_master_db(master_db_path, content_id) {
        Ok(cues) => {
          eprintln!(
            "[Rekordbox] loaded {} cue entries for {}",
            cues.len(),
            path.display()
          );
          meta.cues = cues;
        }
        Err(err) => eprintln!(
          "[Rekordbox] cue read unavailable for {}: {err}",
          path.display()
        ),
      }
    }
    match decode_track_full(&path, override_meta) {
      Ok(t) => {
        eprintln!(
          "[D&D] Decode done deck={} bpm={:?} beats={} title={:?}",
          deck,
          t.bpm,
          t.beats.len(),
          t.title
        );
        let total_frames = (t.pcm.len() / 2) as f32;
        let _ = tx.send(DecodeResult::Ready(DecodeReady {
          deck,
          pcm: t.pcm,
          waveform: t.waveform,
          waveform_colors: t.waveform_colors,
          bpm: t.bpm,
          title: t.title,
          beats: t.beats,
          intro: t.intro,
          outro: t.outro,
          cues: t.cues,
          total_frames,
        }));
      }
      Err(error) => {
        eprintln!("[D&D] Decode failed for deck={}: {}", deck, error);
        let _ = tx.send(DecodeResult::Failed(DecodeFailure { deck, path, error }));
      }
    }
  });
}

fn rekordbox_cues_to_visuals(
  cues: &[sujay_library::RekordboxCue],
  total_frames: usize,
  sample_rate: f32,
) -> Vec<DeckCueVisualState> {
  let duration_ms = total_frames as f32 / sample_rate.max(1.0) * 1000.0;
  if duration_ms <= 0.0 {
    return Vec::new();
  }

  let mut memory_cue_index = 0;
  cues
    .iter()
    .filter_map(|cue| {
      let position = cue.time_ms as f32 / duration_ms;
      if !(0.0..=1.0).contains(&position) {
        return None;
      }
      let loop_end = if cue.is_loop && cue.loop_time_ms > cue.time_ms {
        let end = cue.loop_time_ms as f32 / duration_ms;
        (end <= 1.0).then_some(end)
      } else {
        None
      };
      let label = if cue.hot_cue > 0 {
        cue.hot_cue.to_string()
      } else {
        memory_cue_index += 1;
        format!("M{memory_cue_index}")
      };
      Some(DeckCueVisualState {
        label,
        position,
        loop_end,
        color_rgb: cue.color_rgb.map(|(red, green, blue)| [red, green, blue]),
      })
    })
    .collect()
}
