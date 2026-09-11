use std::env;
use std::path::PathBuf;
use std::time::Instant;

use diesel::prelude::*;
use diesel::sql_types::{Integer, Nullable, Text};

#[derive(QueryableByName)]
struct ContentCueRow {
    #[diesel(sql_type = Text)]
    content_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    cues: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    cue_count: Option<i32>,
}

fn main() {
    let mut args = env::args().skip(1);
    let mut find_colored = false;
    let mut find_cues = false;
    let mut scan_limit: usize = 300;
    let mut show_anlz_paths = false;
    let mut show_anlz_limit: usize = 10;

    let mut path_arg: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--find-colored" => find_colored = true,
            "--find-cues" => find_cues = true,
            "--show-anlz-paths" => show_anlz_paths = true,
            "--scan-limit" => {
                if let Some(v) = args.next() {
                    scan_limit = v.parse::<usize>().unwrap_or(scan_limit);
                }
            }
            "--anlz-limit" => {
                if let Some(v) = args.next() {
                    show_anlz_limit = v.parse::<usize>().unwrap_or(show_anlz_limit);
                }
            }
            _ => {
                if path_arg.is_none() {
                    path_arg = Some(PathBuf::from(arg));
                }
            }
        }
    }

    let start = Instant::now();

    let library = if let Some(path) = path_arg {
        println!("loading Rekordbox library from {}", path.display());
        let loaded = if find_colored || find_cues || show_anlz_paths {
            sujay_library::RekordboxLibrary::load_from_master_db_fast(&path)
        } else {
            sujay_library::RekordboxLibrary::load_from_master_db(&path)
        };
        match loaded {
            Ok(library) => library,
            Err(err) => {
                eprintln!("failed to load library: {}", err);
                std::process::exit(1);
            }
        }
    } else {
        println!("loading Rekordbox library from detected default path");
        let loaded = if find_colored || find_cues || show_anlz_paths {
            sujay_library::RekordboxLibrary::load_default_fast()
        } else {
            sujay_library::RekordboxLibrary::load_default()
        };
        match loaded {
            Ok(library) => library,
            Err(err) => {
                eprintln!("failed to load library: {}", err);
                std::process::exit(1);
            }
        }
    };

    let elapsed = start.elapsed();
    println!("master.db: {}", library.master_db_path.display());
    println!("playlists: {}", library.playlists.len());
    println!("tracks: {}", library.tracks.len());
    println!("elapsed_ms: {}", elapsed.as_millis());

    if show_anlz_paths {
        let mut configured = 0usize;
        let mut scanned = 0usize;
        for (index, track) in library.tracks.iter().take(show_anlz_limit).enumerate() {
            scanned += 1;
            match sujay_library::load_track_analysis_from_master_db(
                &library.master_db_path,
                &track.id,
            ) {
                Ok(analysis) => {
                    if let Some(anlz_path) = analysis.analysis_path {
                        configured += 1;
                        println!(
                            "{:>3}: {} | {} | {} | anlz_path={} | beats={} | waveform={}",
                            index + 1,
                            track.artist,
                            track.album,
                            track.title,
                            anlz_path.display(),
                            analysis.beats_ms.len(),
                            analysis.waveform.len(),
                        );
                    }
                }
                Err(err) => {
                    println!(
                        "{:>3}: {} | {} | {} | analysis_error={}",
                        index + 1,
                        track.artist,
                        track.album,
                        track.title,
                        err,
                    );
                }
            }
        }
        println!("anlz_path_configured: {}", configured);
        println!("anlz_path_scanned: {}", scanned);
        return;
    }

    if find_colored {
        let mut colored_tracks = 0usize;
        let mut scanned = 0usize;
        for track in library.tracks.iter().take(scan_limit) {
            scanned += 1;
            match sujay_library::load_track_analysis_from_master_db(
                &library.master_db_path,
                &track.id,
            ) {
                Ok(analysis) if !analysis.waveform.is_empty() => {
                    colored_tracks += 1;
                    println!(
                        "colored: {} | {} | {} | bpm={:?} | waveform={} | path={}",
                        track.artist,
                        track.album,
                        track.title,
                        track.bpm,
                        analysis.waveform.len(),
                        track.file_path.display(),
                    );
                    if colored_tracks >= 20 {
                        break;
                    }
                }
                _ => {}
            }
        }
        println!("colored_tracks_found: {}", colored_tracks);
        println!("scan_limit: {}", scan_limit);
        println!("scanned: {}", scanned);
        return;
    }

    if find_cues {
        let db = match rbox::MasterDb::new(&library.master_db_path) {
            Ok(db) => db,
            Err(err) => {
                eprintln!("failed to open master.db: {}", err);
                std::process::exit(1);
            }
        };
        let mut connection = match db.pool.get() {
            Ok(connection) => connection,
            Err(err) => {
                eprintln!("failed to acquire master.db connection: {}", err);
                std::process::exit(1);
            }
        };
        let cue_entries = match diesel::sql_query(
            "SELECT ContentID AS content_id, Cues AS cues, rb_cue_count AS cue_count \
             FROM contentCue WHERE rb_cue_count > 0 LIMIT ?",
        )
        .bind::<Integer, _>(scan_limit as i32)
        .load::<ContentCueRow>(&mut connection)
        {
            Ok(entries) => entries,
            Err(err) => {
                eprintln!("failed to read cue entries: {}", err);
                std::process::exit(1);
            }
        };

        for cue_entry in &cue_entries {
            let track = library.track(&cue_entry.content_id);
            let parsed_count = sujay_library::load_track_cues_from_master_db(
                &library.master_db_path,
                &cue_entry.content_id,
            )
            .map(|cues| cues.len())
            .unwrap_or_default();
            println!(
                "cues: {} | db_count={} | parsed_count={} | json={}",
                track.map_or("<missing>", |track| track.title.as_str()),
                cue_entry.cue_count.unwrap_or_default(),
                parsed_count,
                cue_entry.cues.as_deref().unwrap_or("<NULL>"),
            );
        }
        println!("cue_tracks_found: {}", cue_entries.len());
        println!("scan_limit: {}", scan_limit);
        return;
    }

    for (index, track) in library.tracks.iter().take(20).enumerate() {
        println!(
            "{:>3}: {} | {} | {} | bpm={:?} | path={} | beats={} | cues={} | waveform={}",
            index + 1,
            track.artist,
            track.album,
            track.title,
            track.bpm,
            track.file_path.display(),
            track.beats_ms.len(),
            track.cues.len(),
            track.waveform.len(),
        );
    }
}
