use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use diesel::prelude::*;
use diesel::sql_types::{Nullable, Text};
use rbox::anlz::anlz::{CueList, CueStatus, CueType, ExtendedCueList, Waveform3BandColumn};
use rbox::masterdb::models::DjmdContent;
use rbox::MasterDb;
use serde::Deserialize;
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
    pub tags: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RekordboxTrackAnalysis {
    pub analysis_path: Option<PathBuf>,
    pub beats_ms: Vec<f32>,
    pub cues: Vec<RekordboxCue>,
    pub waveform: Vec<RekordboxWaveformSample>,
}

#[derive(QueryableByName)]
struct ContentCueJson {
    #[diesel(sql_type = Nullable<Text>)]
    cues: Option<String>,
}

#[derive(Deserialize)]
struct StoredCue {
    #[serde(rename = "InMsec")]
    in_msec: i64,
    #[serde(rename = "OutMsec")]
    out_msec: Option<i64>,
    #[serde(rename = "ColorTableIndex")]
    color_table_index: Option<i32>,
    #[serde(rename = "Comment")]
    comment: Option<String>,
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
        let mut db =
            MasterDb::new(&master_db_path).map_err(|err| LibraryError::Load(err.to_string()))?;

        let playlists = load_playlists(&mut db)?;
        let tracks = load_tracks(&mut db, true)?;

        Ok(Self {
            master_db_path,
            playlists,
            tracks,
        })
    }

    pub fn load_from_master_db_fast<P: AsRef<Path>>(
        master_db_path: P,
    ) -> Result<Self, LibraryError> {
        let master_db_path = master_db_path.as_ref().to_path_buf();
        let mut db =
            MasterDb::new(&master_db_path).map_err(|err| LibraryError::Load(err.to_string()))?;

        let playlists = load_playlists(&mut db)?;
        let tracks = load_tracks(&mut db, false)?;

        Ok(Self {
            master_db_path,
            playlists,
            tracks,
        })
    }

    pub fn track(&self, id: &str) -> Option<&RekordboxTrack> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn playlist(&self, id: &str) -> Option<&RekordboxPlaylist> {
        self.playlists.iter().find(|playlist| playlist.id == id)
    }
}

pub fn load_track_analysis_default(
    content_id: &str,
) -> Result<RekordboxTrackAnalysis, LibraryError> {
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

/// Load only a track's ANLZ beat grid.
///
/// The desktop app fetches this lazily when a library track is loaded into a
/// deck, keeping initial library listing fast while retaining beat markers.
pub fn load_track_beats_from_master_db<P: AsRef<Path>>(
    master_db_path: P,
    content_id: &str,
) -> Result<Vec<f32>, LibraryError> {
    let mut db = MasterDb::new(master_db_path.as_ref())
        .map_err(|err| LibraryError::Load(err.to_string()))?;
    let Some(mut files) = db
        .get_content_anlz_files(content_id)
        .map_err(|err| LibraryError::Load(err.to_string()))?
    else {
        return Ok(Vec::new());
    };
    Ok(files
        .dat
        .get_beat_grid()
        .map(|beat_grid| {
            beat_grid
                .beats
                .iter()
                .map(|beat| beat.time as f32)
                .collect()
        })
        .unwrap_or_default())
}

/// Load the cue and loop entries from one track's ANLZ data.
pub fn load_track_cues_from_master_db<P: AsRef<Path>>(
    master_db_path: P,
    content_id: &str,
) -> Result<Vec<RekordboxCue>, LibraryError> {
    let mut db = MasterDb::new(master_db_path.as_ref())
        .map_err(|err| LibraryError::Load(err.to_string()))?;
    let mut connection = db
        .pool
        .get()
        .map_err(|err| LibraryError::Load(err.to_string()))?;
    let cue_rows = diesel::sql_query(
        "SELECT Cues AS cues FROM contentCue WHERE ContentID = ?",
    )
    .bind::<Text, _>(content_id)
    .load::<ContentCueJson>(&mut connection)
    .map_err(|err| LibraryError::Load(err.to_string()))?;

    let mut cues = Vec::new();
    for row in cue_rows {
        if let Some(json) = row.cues {
            cues.extend(parse_stored_cues(&json)?);
        }
    }

    // Older Rekordbox exports store cue data only in the ANLZ file.
    cues.extend(extract_track_analysis(&mut db, content_id).cues);
    cues.sort_by_key(|cue| (cue.time_ms, cue.hot_cue));
    cues.dedup_by(|left, right| {
        left.time_ms == right.time_ms
            && left.loop_time_ms == right.loop_time_ms
            && left.is_loop == right.is_loop
    });
    Ok(cues)
}

fn parse_stored_cues(json: &str) -> Result<Vec<RekordboxCue>, LibraryError> {
    Ok(serde_json::from_str::<Vec<StoredCue>>(json)
        .map_err(|err| LibraryError::Load(format!("invalid contentCue JSON: {err}")))?
        .into_iter()
        .enumerate()
        .filter_map(|(index, cue)| {
            let time_ms = u32::try_from(cue.in_msec).ok()?;
            let loop_time_ms = cue
                .out_msec
                .and_then(|out_msec| u32::try_from(out_msec).ok())
                .filter(|out_msec| *out_msec > time_ms)
                .unwrap_or_default();
            Some(RekordboxCue {
                hot_cue: index as u32 + 1,
                time_ms,
                loop_time_ms,
                is_loop: loop_time_ms > 0,
                color_rgb: rekordbox_cue_color(cue.color_table_index),
                comment: cue.comment.filter(|comment| !comment.is_empty()),
            })
        })
        .collect())
}

fn rekordbox_cue_color(color_table_index: Option<i32>) -> Option<(u8, u8, u8)> {
    const COLORS: [(u8, u8, u8); 14] = [
        (0xCC, 0x00, 0x00),
        (0xCC, 0x44, 0x00),
        (0xCC, 0x88, 0x00),
        (0xCC, 0xCC, 0x00),
        (0x88, 0xCC, 0x00),
        (0x00, 0xCC, 0x00),
        (0x00, 0xCC, 0x88),
        (0x00, 0xCC, 0xCC),
        (0x00, 0x88, 0xCC),
        (0x00, 0x00, 0xCC),
        (0x88, 0x00, 0xCC),
        (0xCC, 0x00, 0xCC),
        (0xCC, 0x00, 0x88),
        (0xFF, 0xFF, 0xFF),
    ];
    color_table_index
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| COLORS.get(index as usize).copied())
}

fn load_tracks(
    db: &mut MasterDb,
    include_analysis: bool,
) -> Result<Vec<RekordboxTrack>, LibraryError> {
    let contents = db
        .get_contents()
        .map_err(|err| LibraryError::Load(err.to_string()))?;

    let mut tracks = Vec::with_capacity(contents.len());
    let mut artist_name_cache: HashMap<String, String> = HashMap::new();
    let mut album_name_cache: HashMap<String, String> = HashMap::new();
    for content in contents {
        if let Some(track) = content_to_track(
            db,
            content,
            include_analysis,
            &mut artist_name_cache,
            &mut album_name_cache,
        ) {
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

fn content_to_track(
    db: &mut MasterDb,
    content: DjmdContent,
    include_analysis: bool,
    artist_name_cache: &mut HashMap<String, String>,
    album_name_cache: &mut HashMap<String, String>,
) -> Option<RekordboxTrack> {
    let file_path = resolve_audio_path(&content)?;
    let analysis = if include_analysis {
        extract_track_analysis(db, &content.id)
    } else {
        RekordboxTrackAnalysis::default()
    };

    let artist = resolve_artist_name(db, &content, artist_name_cache);
    let album = resolve_album_name(db, &content, album_name_cache);

    Some(RekordboxTrack {
        id: content.id,
        title: content
            .title
            .or(content.file_name_l.clone())
            .unwrap_or_else(|| "Untitled".to_owned()),
        artist,
        album,
        bpm: content
            .bpm
            .map(|b| if b > 300 { b as f32 / 100.0 } else { b as f32 }),
        duration_seconds: content.length.map(|ms| ms as f32 / 1000.0),
        file_path,
        analysis_path: analysis.analysis_path,
        beats_ms: analysis.beats_ms,
        cues: analysis.cues,
        waveform: analysis.waveform,
        track_no: content.track_no,
        rating: content.rating,
        tags: content.tag,
        release_date: content.release_date.or(content.date_created),
    })
}

fn resolve_artist_name(
    db: &mut MasterDb,
    content: &DjmdContent,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(name) = content
        .src_artist_name
        .as_ref()
        .filter(|name| !name.is_empty())
    {
        return name.clone();
    }

    let Some(artist_id) = content.artist_id.as_ref().filter(|id| !id.is_empty()) else {
        return String::new();
    };

    if let Some(cached) = cache.get(artist_id) {
        return cached.clone();
    }

    let name = db
        .get_artist_by_id(artist_id)
        .ok()
        .flatten()
        .map(|artist| artist.name)
        .unwrap_or_default();
    cache.insert(artist_id.clone(), name.clone());
    name
}

fn resolve_album_name(
    db: &mut MasterDb,
    content: &DjmdContent,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(name) = content
        .src_album_name
        .as_ref()
        .filter(|name| !name.is_empty())
    {
        return name.clone();
    }

    let Some(album_id) = content.album_id.as_ref().filter(|id| !id.is_empty()) else {
        return String::new();
    };

    if let Some(cached) = cache.get(album_id) {
        return cached.clone();
    }

    let name = db
        .get_album_by_id(album_id)
        .ok()
        .flatten()
        .map(|album| album.name)
        .unwrap_or_default();
    cache.insert(album_id.clone(), name.clone());
    name
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
            beats_ms = beat_grid
                .beats
                .iter()
                .map(|beat| beat.time as f32)
                .collect();
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
        } else if let Some(preview) = anlz.get_waveform_color_preview() {
            // Some tracks expose only PWV4 preview data (band energies) and not
            // PWV5 true-color detail. Map band energies to pseudo-RGB so the UI
            // can still render a colored waveform instead of falling back to white.
            waveform = preview
                .data
                .iter()
                .map(|column| {
                    let r = column.energy_bottom_third_freq;
                    let g = column.energy_mid_third_freq;
                    let b = column.energy_top_third_freq;
                    let h = r.max(g).max(b).max(column.energy_bottom_half_freq);
                    RekordboxWaveformSample {
                        red: r,
                        green: g,
                        blue: b,
                        height: h,
                    }
                })
                .collect();
        } else if let Some(detail) = anlz.get_waveform_3band_detail() {
            waveform = detail
                .data
                .iter()
                .map(three_band_column_to_waveform_sample)
                .collect();
        } else if let Some(preview) = anlz.get_waveform_3band_preview() {
            waveform = preview
                .data
                .iter()
                .map(three_band_column_to_waveform_sample)
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

fn three_band_column_to_waveform_sample(column: &Waveform3BandColumn) -> RekordboxWaveformSample {
    let low = column.low();
    let mid = column.mid();
    let high = column.high();
    let height = low.max(mid).max(high);

    // Rekordbox 3-band waveform encodes low / mid / high energy rather than
    // explicit RGB values. Map those bands to a stable blue → amber → white
    // palette so the UI still shows a colored waveform for tracks that only
    // ship PWV6/PWV7 data.
    let red = mid.saturating_add(high / 2);
    let green = mid.saturating_add(high / 3);
    let blue = low.saturating_add(high / 2);

    RekordboxWaveformSample {
        red,
        green,
        blue,
        height,
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
        list.cues
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
        list.cues
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
