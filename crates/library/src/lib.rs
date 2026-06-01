#[cfg(target_os = "windows")]
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rbox::anlz::anlz::{CueList, CueStatus, CueType, ExtendedCueList};
use rbox::masterdb::models::DjmdContent;
use rbox::MasterDb;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("no Rekordbox master.db found")]
    NotFound,
    #[error("failed to load Rekordbox library: {0}")]
    Load(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxCue {
    pub hot_cue: u32,
    pub time_ms: u32,
    pub loop_time_ms: u32,
    pub is_loop: bool,
    pub color_rgb: Option<(u8, u8, u8)>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxWaveformSample {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub height: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub bpm: Option<f32>,
    pub duration_seconds: Option<f32>,
    pub file_path: PathBuf,
    pub analysis_path: Option<PathBuf>,
    pub beats_ms: Vec<f32>,
    pub cues: Vec<RekordboxCue>,
    pub waveform: Vec<RekordboxWaveformSample>,
    pub track_no: Option<i32>,
    pub rating: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxTrackAnalysis {
    pub analysis_path: Option<PathBuf>,
    pub beats_ms: Vec<f32>,
    pub cues: Vec<RekordboxCue>,
    pub waveform: Vec<RekordboxWaveformSample>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxPlaylist {
    pub id: String,
    pub name: String,
    pub parent_id: String,
    pub is_folder: bool,
    pub track_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxLibrary {
    pub master_db_path: PathBuf,
    pub playlists: Vec<RekordboxPlaylist>,
    pub tracks: Vec<RekordboxTrack>,
}

impl RekordboxLibrary {
    pub fn load_default() -> Result<Self, LibraryError> {
        let master_db_path = detect_master_db_path().ok_or(LibraryError::NotFound)?;
        Self::load_from_master_db(master_db_path)
    }

    pub fn load_default_fast() -> Result<Self, LibraryError> {
        let master_db_path = detect_master_db_path().ok_or(LibraryError::NotFound)?;
        Self::load_from_master_db_fast(master_db_path)
    }

    pub fn load_from_master_db<P: AsRef<Path>>(master_db_path: P) -> Result<Self, LibraryError> {
        let master_db_path = master_db_path.as_ref().to_path_buf();
        let mut db = MasterDb::new(&master_db_path)
            .map_err(|err| LibraryError::Load(err.to_string()))?;

        let playlists = load_playlists(&mut db)?;
        let tracks = load_tracks(&mut db, true)?;

        Ok(Self { master_db_path, playlists, tracks })
    }

    pub fn load_from_master_db_fast<P: AsRef<Path>>(master_db_path: P) -> Result<Self, LibraryError> {
        let master_db_path = master_db_path.as_ref().to_path_buf();
        let mut db = MasterDb::new(&master_db_path)
            .map_err(|err| LibraryError::Load(err.to_string()))?;

        let playlists = load_playlists(&mut db)?;
        let tracks = load_tracks(&mut db, false)?;

        Ok(Self { master_db_path, playlists, tracks })
    }

    pub fn track(&self, id: &str) -> Option<&RekordboxTrack> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn playlist(&self, id: &str) -> Option<&RekordboxPlaylist> {
        self.playlists.iter().find(|playlist| playlist.id == id)
    }
}

pub fn load_track_analysis_default(content_id: &str) -> Result<RekordboxTrackAnalysis, LibraryError> {
    let master_db_path = detect_master_db_path().ok_or(LibraryError::NotFound)?;
    load_track_analysis_from_master_db(master_db_path, content_id)
}

pub fn load_track_analysis_from_master_db<P: AsRef<Path>>(
    master_db_path: P,
    content_id: &str,
) -> Result<RekordboxTrackAnalysis, LibraryError> {
    let mut db = MasterDb::new(master_db_path.as_ref())
        .map_err(|err| LibraryError::Load(err.to_string()))?;

    Ok(extract_track_analysis(&mut db, content_id))
}

fn load_tracks(db: &mut MasterDb, include_analysis: bool) -> Result<Vec<RekordboxTrack>, LibraryError> {
    let contents = db
        .get_contents()
        .map_err(|err| LibraryError::Load(err.to_string()))?;

    let mut tracks = Vec::with_capacity(contents.len());
    for content in contents {
        if let Some(track) = content_to_track(db, content, include_analysis) {
            tracks.push(track);
        }
    }
    Ok(tracks)
}

fn load_playlists(db: &mut MasterDb) -> Result<Vec<RekordboxPlaylist>, LibraryError> {
    let playlists = db
        .get_playlists()
        .map_err(|err| LibraryError::Load(err.to_string()))?;

    let mut result = Vec::with_capacity(playlists.len());
    for playlist in playlists {
        let track_ids = db
            .get_playlist_contents(&playlist.id)
            .map_err(|err| LibraryError::Load(err.to_string()))?
            .into_iter()
            .map(|content| content.id)
            .collect();

        result.push(RekordboxPlaylist {
            id: playlist.id,
            name: playlist.name,
            parent_id: playlist.parent_id,
            is_folder: playlist.attribute == 1,
            track_ids,
        });
    }

    Ok(result)
}

fn content_to_track(db: &mut MasterDb, content: DjmdContent, include_analysis: bool) -> Option<RekordboxTrack> {
    let file_path = resolve_audio_path(&content)?;
    let analysis = if include_analysis {
        extract_track_analysis(db, &content.id)
    } else {
        RekordboxTrackAnalysis::default()
    };

    Some(RekordboxTrack {
        id: content.id,
        title: content.title.or(content.file_name_l.clone()).unwrap_or_else(|| "Untitled".to_owned()),
        artist: content.src_artist_name.unwrap_or_default(),
        album: content.src_album_name.unwrap_or_default(),
        bpm: content.bpm.map(|b| if b > 300 { b as f32 / 100.0 } else { b as f32 }),
        duration_seconds: content.length.map(|ms| ms as f32 / 1000.0),
        file_path,
        analysis_path: analysis.analysis_path,
        beats_ms: analysis.beats_ms,
        cues: analysis.cues,
        waveform: analysis.waveform,
        track_no: content.track_no,
        rating: content.rating,
    })
}

fn extract_track_analysis(db: &mut MasterDb, content_id: &str) -> RekordboxTrackAnalysis {
    let analysis_path = db
        .get_content_anlz_paths(content_id)
        .ok()
        .flatten()
        .map(|paths| paths.dat);

    let mut beats_ms = Vec::new();
    let mut cues = Vec::new();
    let mut waveform = Vec::new();

    if let Ok(Some(mut files)) = db.get_content_anlz_files(content_id) {
        let anlz = &mut files.dat;
        if let Some(beat_grid) = anlz.get_beat_grid() {
            beats_ms = beat_grid.beats.iter().map(|beat| beat.time as f32).collect();
        }
        cues.extend(load_cue_list(anlz.get_extended_hot_cues(), true));
        cues.extend(load_cue_list(anlz.get_extended_memory_cues(), false));
        cues.extend(load_cue_list_legacy(anlz.get_hot_cues(), true));
        cues.extend(load_cue_list_legacy(anlz.get_memory_cues(), false));
        if let Some(detail) = anlz.get_waveform_color_detail() {
            waveform = detail
                .data
                .iter()
                .map(|column| RekordboxWaveformSample {
                    red: column.red(),
                    green: column.green(),
                    blue: column.blue(),
                    height: column.height(),
                })
                .collect();
        }
    }

    RekordboxTrackAnalysis {
        analysis_path,
        beats_ms,
        cues,
        waveform,
    }
}

fn resolve_audio_path(content: &DjmdContent) -> Option<PathBuf> {
    let folder = content
        .rb_local_folder_path
        .as_deref()
        .or(content.folder_path.as_deref())
        .or(content.org_folder_path.as_deref())?;

    let folder_path = Path::new(folder);

    // Rekordbox stores the full track path in folder_path in many databases,
    // even though the field name sounds directory-like. If it already looks
    // like a file path, keep it as-is instead of joining file_name again.
    if folder_path.is_file() || folder_path.extension().is_some() {
        return Some(folder_path.to_path_buf());
    }

    let file_name = content
        .file_name_l
        .as_deref()
        .or(content.file_name_s.as_deref())?;

    Some(folder_path.join(file_name))
}

fn load_cue_list(list: Option<&mut ExtendedCueList>, hot: bool) -> Vec<RekordboxCue> {
    list.map(|list| {
        list
            .cues
            .iter()
            .map(|cue| RekordboxCue {
                hot_cue: cue.hot_cue,
                time_ms: cue.time,
                loop_time_ms: cue.loop_time,
                is_loop: hot && matches!(cue.cue_type, CueType::Loop),
                color_rgb: Some(cue.hot_cue_color_rgb),
                comment: Some(cue.comment.to_string()),
            })
            .collect()
    })
    .unwrap_or_default()
}

fn load_cue_list_legacy(list: Option<&mut CueList>, hot: bool) -> Vec<RekordboxCue> {
    list.map(|list| {
        list
            .cues
            .iter()
            .map(|cue| RekordboxCue {
                hot_cue: cue.hot_cue,
                time_ms: cue.time,
                loop_time_ms: cue.loop_time,
                is_loop: hot && matches!(cue.status, CueStatus::Active),
                color_rgb: None,
                comment: None,
            })
            .collect()
    })
    .unwrap_or_default()
}

fn detect_master_db_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        let pioneer_dir = home.join("Library/Pioneer");
        find_newest_master_db(&pioneer_dir)
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var_os("APPDATA").map(PathBuf::from)?;
        let pioneer_dir = appdata.join("Pioneer");
        find_newest_master_db(&pioneer_dir)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = env::var_os("APPDATA");
        None
    }
}

fn find_newest_master_db(base_dir: &Path) -> Option<PathBuf> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(base_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        if !name.starts_with("rekordbox") {
            continue;
        }
        let candidate = path.join("master.db");
        let metadata = match candidate.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        match newest {
            Some((current, _)) if current >= modified => {}
            _ => newest = Some((modified, candidate)),
        }
    }
    newest.map(|(_, path)| path)
}
