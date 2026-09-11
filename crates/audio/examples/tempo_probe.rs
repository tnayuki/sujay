//! Diagnostic: does master tempo change the playback rate when a deck has a BPM?
//! `cargo run -p sujay_audio --example tempo_probe -- <wav>`
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sujay_audio::engine_core::{AudioEngineCore, EngineStateUpdate};

fn main() {
  let path = std::env::args().nth(1).expect("wav path");
  let last: Arc<Mutex<Option<EngineStateUpdate>>> = Arc::new(Mutex::new(None));
  let sink = Arc::clone(&last);
  let engine = AudioEngineCore::new(Some(44100), Arc::new(move |s| { *sink.lock().unwrap() = Some(s); })).unwrap();
  let decoded = sujay_audio::decoder::decode_audio(path, 44100, 2).unwrap();
  let mut pcm = Some(decoded.pcm);
  let mut id = Some("probe".to_string());
  while !engine.try_load_track(1, &mut pcm, Some(120.0), vec![], &mut id) {
    std::thread::sleep(Duration::from_millis(5));
  }
  engine.play(1).unwrap();
  std::thread::sleep(Duration::from_millis(300));
  let position = || last.lock().unwrap().as_ref().and_then(|s| s.deck_a_position).unwrap_or(0.0);
  for tempo in [120.0, 240.0, 60.0] {
    engine.set_master_tempo(tempo).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let p0 = position(); let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(1000));
    let dt = t0.elapsed().as_secs_f64();
    let bpm = last.lock().unwrap().as_ref().and_then(|s| s.deck_a_bpm);
    println!("master={tempo} deck_bpm={bpm:?} rate={:.2}x", (position() - p0) / 44100.0 / dt);
  }
  engine.close();
}
