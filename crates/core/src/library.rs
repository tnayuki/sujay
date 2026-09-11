//! Rekordbox library: background load of the browse list plus the path-keyed
//! metadata that lets a file dropped from Finder pick up its rekordbox BPM,
//! beat grid and cues.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::state::{LibraryPlaylistItem, LibraryTrackItem};

/// Metadata rekordbox knows about a file, applied on top of the decoder's own.
#[derive(Clone, Debug, Default)]
pub struct RekordboxTrackOverride {
  pub title: String,
  pub bpm: Option<f32>,
  pub beats_ms: Vec<f32>,
  pub waveform: Vec<f32>,
  pub waveform_colors: Vec<[u8; 3]>,
  pub cues: Vec<sujay_library::RekordboxCue>,
}

pub struct RekordboxLibraryLoadReady {
  pub master_db_path: PathBuf,
  pub source_label: String,
  pub tracks: Vec<LibraryTrackItem>,
  pub playlists: Vec<LibraryPlaylistItem>,
  pub overrides: HashMap<PathBuf, RekordboxTrackOverride>,
  pub track_ids: HashMap<PathBuf, String>,
}

pub type LibraryLoadResult = Result<RekordboxLibraryLoadReady, String>;

pub fn normalize_track_path(path: &Path) -> PathBuf {
  fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Load the browse list (no per-track ANLZ) on a background thread.
pub fn spawn_rekordbox_library_load(tx: Sender<LibraryLoadResult>) {
  std::thread::spawn(move || {
    let loaded = sujay_library::RekordboxLibrary::load_default_fast()
      .map_err(|err| err.to_string())
      .map(|library| {
        let mut overrides = HashMap::with_capacity(library.tracks.len());
        let mut track_ids = HashMap::with_capacity(library.tracks.len());
        let mut tracks = Vec::with_capacity(library.tracks.len());
        let playlists = library
          .playlists
          .iter()
          .map(|playlist| LibraryPlaylistItem {
            id: playlist.id.clone(),
            name: playlist.name.clone(),
            parent_id: playlist.parent_id.clone(),
            is_folder: playlist.is_folder,
            track_ids: playlist.track_ids.clone(),
          })
          .collect();

        for track in library.tracks {
          let override_value = RekordboxTrackOverride {
            title: track.title.clone(),
            bpm: track.bpm,
            beats_ms: track.beats_ms.clone(),
            waveform: vec![],
            waveform_colors: track
              .waveform
              .iter()
              .map(|sample| [sample.red, sample.green, sample.blue])
              .collect(),
            cues: track.cues.clone(),
          };

          let raw_key = track.file_path.clone();
          track_ids.insert(raw_key.clone(), track.id.clone());
          overrides.insert(raw_key.clone(), override_value.clone());

          let normalized_key = normalize_track_path(&track.file_path);
          if normalized_key != raw_key {
            track_ids.insert(normalized_key.clone(), track.id.clone());
            overrides.insert(normalized_key, override_value);
          }

          tracks.push(LibraryTrackItem {
            id: track.id,
            title: track.title,
            artist: track.artist,
            album: track.album,
            bpm: track.bpm,
            duration_seconds: track.duration_seconds,
            rating: track.rating,
            tags: track.tags,
            release_date: track.release_date,
            file_path: track.file_path.to_string_lossy().to_string(),
          });
        }

        RekordboxLibraryLoadReady {
          master_db_path: library.master_db_path.clone(),
          source_label: library.master_db_path.to_string_lossy().to_string(),
          tracks,
          playlists,
          overrides,
          track_ids,
        }
      });
    let _ = tx.send(loaded);
  });
}
