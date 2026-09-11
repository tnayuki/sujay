//! C ABI over the one thing still in Rust: the rekordbox reader
//! (`sujay_library`). The audio engine and everything else live in Swift.
//!
//! Strings returned as `char *` are freed with `sujay_string_free`. The
//! header is hand-written: `Vendor/SujayCore/include/sujay.h`; keep the two
//! in step.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, CStr, CString};

use serde::Serialize;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn to_c_string(s: String) -> *mut c_char {
  CString::new(s)
    .map(CString::into_raw)
    .unwrap_or(std::ptr::null_mut())
}

unsafe fn c_str<'a>(ptr: *const c_char) -> Option<&'a str> {
  if ptr.is_null() {
    return None;
  }
  CStr::from_ptr(ptr).to_str().ok()
}

fn json<T: Serialize>(value: &T) -> *mut c_char {
  to_c_string(serde_json::to_string(value).unwrap_or_default())
}

#[no_mangle]
pub unsafe extern "C" fn sujay_string_free(s: *mut c_char) {
  if !s.is_null() {
    drop(CString::from_raw(s));
  }
}

// ── Rekordbox ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TrackJson {
  id: String,
  title: String,
  artist: String,
  album: String,
  bpm: Option<f32>,
  duration_seconds: Option<f32>,
  rating: Option<i32>,
  tags: Option<String>,
  release_date: Option<String>,
  file_path: String,
}

#[derive(Serialize)]
struct PlaylistJson {
  id: String,
  name: String,
  parent_id: String,
  is_folder: bool,
  track_ids: Vec<String>,
}

#[derive(Serialize)]
struct LibraryJson {
  master_db_path: String,
  tracks: Vec<TrackJson>,
  playlists: Vec<PlaylistJson>,
}

#[derive(Serialize)]
struct ErrorJson {
  error: String,
}

/// The browse list without per-track analysis. `master_db` may be NULL to
/// find the newest rekordbox `master.db` under ~/Library/Pioneer. On failure
/// the JSON is `{"error": "..."}`.
#[no_mangle]
pub unsafe extern "C" fn sujay_library_load_json(master_db: *const c_char) -> *mut c_char {
  let loaded = match c_str(master_db) {
    Some(path) => sujay_library::RekordboxLibrary::load_from_master_db_fast(path),
    None => sujay_library::RekordboxLibrary::load_default_fast(),
  };
  match loaded {
    Ok(library) => json(&LibraryJson {
      master_db_path: library.master_db_path.to_string_lossy().to_string(),
      tracks: library
        .tracks
        .into_iter()
        .map(|t| TrackJson {
          id: t.id,
          title: t.title,
          artist: t.artist,
          album: t.album,
          bpm: t.bpm,
          duration_seconds: t.duration_seconds,
          rating: t.rating,
          tags: t.tags,
          release_date: t.release_date,
          file_path: t.file_path.to_string_lossy().to_string(),
        })
        .collect(),
      playlists: library
        .playlists
        .into_iter()
        .map(|p| PlaylistJson {
          id: p.id,
          name: p.name,
          parent_id: p.parent_id,
          is_folder: p.is_folder,
          track_ids: p.track_ids,
        })
        .collect(),
    }),
    Err(err) => json(&ErrorJson {
      error: err.to_string(),
    }),
  }
}

#[derive(Serialize)]
struct CueJson {
  hot_cue: u32,
  time_ms: u32,
  loop_time_ms: u32,
  is_loop: bool,
  color_rgb: Option<[u8; 3]>,
  comment: Option<String>,
}

#[derive(Serialize)]
struct AnalysisJson {
  /// Beat positions in milliseconds.
  beats_ms: Vec<f32>,
  cues: Vec<CueJson>,
  /// RGB triplets of the 3-band waveform, flattened.
  waveform_rgb: Vec<u8>,
}

/// One track's beat grid, cues and 3-band waveform colours.
#[no_mangle]
pub unsafe extern "C" fn sujay_library_track_analysis_json(
  master_db: *const c_char,
  content_id: *const c_char,
) -> *mut c_char {
  let (Some(master_db), Some(content_id)) = (c_str(master_db), c_str(content_id)) else {
    return json(&ErrorJson {
      error: "missing arguments".into(),
    });
  };
  let analysis = match sujay_library::load_track_analysis_from_master_db(master_db, content_id) {
    Ok(a) => a,
    Err(err) => {
      return json(&ErrorJson {
        error: err.to_string(),
      })
    }
  };
  // The cue reader merges the database's cue table with the ANLZ cues; the
  // analysis alone has only the latter.
  let cues = sujay_library::load_track_cues_from_master_db(master_db, content_id)
    .unwrap_or(analysis.cues.clone());
  json(&AnalysisJson {
    beats_ms: analysis.beats_ms,
    cues: cues
      .into_iter()
      .map(|c| CueJson {
        hot_cue: c.hot_cue,
        time_ms: c.time_ms,
        loop_time_ms: c.loop_time_ms,
        is_loop: c.is_loop,
        color_rgb: c.color_rgb.map(|(r, g, b)| [r, g, b]),
        comment: c.comment,
      })
      .collect(),
    waveform_rgb: analysis
      .waveform
      .iter()
      .flat_map(|s| [s.red, s.green, s.blue])
      .collect(),
  })
}
