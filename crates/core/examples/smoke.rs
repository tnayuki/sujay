//! Drive `Core` without a window: load a file into deck A, play, loop, EQ,
//! crossfade, stop. A quick end-to-end check of the host orchestration
//! against the real audio engine.
//!
//! Usage: `cargo run -p sujay-core --example smoke -- <audio file>`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sujay_core::Core;

/// Tick until `until` holds, or panic after 60 s.
fn pump(core: &mut Core, until: impl Fn(&Core) -> bool, what: &str) {
  let started = Instant::now();
  loop {
    let tick = core.tick();
    if until(core) {
      println!("ok: {what} ({:.1}s)", started.elapsed().as_secs_f32());
      return;
    }
    if started.elapsed() > Duration::from_secs(60) {
      panic!("timeout waiting for {what}");
    }
    std::thread::sleep(Duration::from_millis(if tick.retry_soon { 1 } else { 20 }));
  }
}

fn main() {
  let path = PathBuf::from(std::env::args().nth(1).expect("usage: smoke <audio file>"));
  let mut core = Core::new();
  core.start().expect("audio engine");
  pump(
    &mut core,
    |c| !c.library_state().tracks.is_empty(),
    "library loaded",
  );
  println!("library: {} tracks", core.library_state().tracks.len());

  core.load_file(1, path);
  pump(
    &mut core,
    |c| !c.deck(1).waveform.is_empty(),
    "deck A waveform",
  );
  let deck = core.deck(1);
  println!(
    "deck A: waveform={} colors={} beats={} progress={:?}",
    deck.waveform.len(),
    deck.waveform_colors.len(),
    deck.beats.len(),
    deck.progress
  );
  pump(
    &mut core,
    |c| c.console_state().deck_a.title != "---",
    "deck A title",
  );
  let deck_a = &core.console_state().deck_a;
  println!(
    "title={:?} bpm={} cues={}",
    deck_a.title,
    deck_a.bpm_text,
    deck_a.rekordbox_cues.len()
  );

  core.play(1);
  pump(&mut core, |c| c.console_state().deck_a.playing, "playing");
  let start_pos = core.deck(1).progress.map(|p| p.0).unwrap_or(0.0);
  std::thread::sleep(Duration::from_millis(500));
  pump(
    &mut core,
    |c| c.deck(1).progress.map(|p| p.0).unwrap_or(0.0) > start_pos + 1000.0,
    "position advancing",
  );

  core.toggle_loop(1, 4.0);
  pump(
    &mut core,
    |c| c.console_state().deck_a.loop_enabled,
    "loop enabled",
  );
  let deck_a = &core.console_state().deck_a;
  println!(
    "loop beats={} start={} end={}",
    deck_a.loop_beats, deck_a.loop_start, deck_a.loop_end
  );
  core.toggle_loop(1, 0.0);
  pump(
    &mut core,
    |c| !c.console_state().deck_a.loop_enabled,
    "loop cleared",
  );

  core.set_eq(1, "low", true);
  pump(
    &mut core,
    |c| c.console_state().deck_a.eq_low,
    "eq low kill",
  );
  core.set_crossfader(0.9);
  pump(
    &mut core,
    |c| (c.console_state().crossfader - 0.9).abs() < 0.01,
    "crossfader",
  );

  core.stop(1);
  pump(&mut core, |c| !c.console_state().deck_a.playing, "stopped");
  core.shutdown();
  println!("done");
}
