use std::env;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args = env::args().skip(1);
    let start = Instant::now();

    let library = if let Some(path) = args.next() {
        let path = PathBuf::from(path);
        println!("loading Rekordbox library from {}", path.display());
        match sujay_library::RekordboxLibrary::load_from_master_db(&path) {
            Ok(library) => library,
            Err(err) => {
                eprintln!("failed to load library: {}", err);
                std::process::exit(1);
            }
        }
    } else {
        println!("loading Rekordbox library from detected default path");
        match sujay_library::RekordboxLibrary::load_default() {
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

    for (index, track) in library.tracks.iter().take(20).enumerate() {
        println!(
            "{:>3}: {} | {} | bpm={:?} | path={} | beats={} | cues={} | waveform={}",
            index + 1,
            track.artist,
            track.title,
            track.bpm,
            track.file_path.display(),
            track.beats_ms.len(),
            track.cues.len(),
            track.waveform.len(),
        );
    }
}