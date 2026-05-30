//! Sujay — Phase 5 Rust-native host binary.
//!
//! Creates a winit window, attaches the wgpu/egui UI renderer, and drives the
//! Rust AudioEngineCore — no Electron, no Node.js, no NAPI.

use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use tracing_subscriber::EnvFilter;
use winit::{
    application::ApplicationHandler,
    event::{WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use sujay_audio::engine_core::{AudioEngineCore, EngineStateUpdate};
use sujay_ui::{
    attach_raw, detach_raw, set_frame_raw, poll_actions_raw, push_mouse_event_raw,
    set_console_state_raw, set_deck_progress_raw,
};

// ── App state ────────────────────────────────────────────────────────────────

/// Result of a background decode, sent back to the main thread for loading.
struct DecodeReady {
    deck: u8,
    pcm: Vec<f32>,
    waveform: Vec<f32>,
    bpm: Option<f32>,
    title: String,
    /// Beat positions in audio frames.
    beats: Vec<f32>,
    /// Intro position in audio frames (if detected).
    intro: Option<f32>,
    /// Outro position in audio frames (if detected).
    outro: Option<f32>,
    /// Total mono frames (pcm.len() / 2).
    total_frames: f32,
}

struct SujayApp {
    window: Option<Arc<Window>>,
    engine: Option<Arc<AudioEngineCore>>,
    /// Latest state update from audio engine (shared with the audio callback).
    last_state: Arc<Mutex<Option<EngineStateUpdate>>>,
    /// Last known cursor position in logical points (top-left origin).
    cursor_pos: (f32, f32),
    /// Deck selected during hover phase (1=A, 2=B).
    hovered_deck: u8,
    /// Sender half for background decode results.
    decode_tx: Sender<DecodeReady>,
    /// Receiver half for background decode results.
    decode_rx: Receiver<DecodeReady>,
}

impl SujayApp {
    fn new() -> Self {
        let (decode_tx, decode_rx) = mpsc::channel();
        Self {
            window: None,
            engine: None,
            last_state: Arc::new(Mutex::new(None)),
            cursor_pos: (0.0, 0.0),
            hovered_deck: 1,
            decode_tx,
            decode_rx,
        }
    }
}

impl ApplicationHandler for SujayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        let attrs = Window::default_attributes()
            .with_title("Sujay")
            .with_inner_size(winit::dpi::LogicalSize::new(1100u32, 760u32));

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let scale = window.scale_factor();
        let logical = window.inner_size().to_logical::<f64>(scale);

        // ── Attach native UI renderer ────────────────────────────────────────
        #[cfg(target_os = "macos")]
        {
            let ns_view = match window.window_handle().unwrap().as_raw() {
                RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut std::ffi::c_void,
                _ => panic!("unexpected window handle type"),
            };
            attach_raw(ns_view, 0.0, 0.0, logical.width, logical.height);
        }

        // ── Start audio engine ───────────────────────────────────────────────
        let last_state = Arc::clone(&self.last_state);
        let engine = AudioEngineCore::new(
            Some(44100),
            Arc::new(move |state: EngineStateUpdate| {
                if let Ok(mut guard) = last_state.lock() { *guard = Some(state); }
            }),
        )
        .expect("Failed to initialise audio engine");

        self.window = Some(window);
        self.engine = Some(Arc::new(engine));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("window close requested");
                if let Some(engine) = self.engine.take() { engine.close(); }
                detach_raw();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let logical = size.to_logical::<f64>(scale);
                set_frame_raw(0.0, 0.0, logical.width, logical.height);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let logical = position.to_logical::<f32>(scale);
                self.cursor_pos = (logical.x, logical.y);
                push_mouse_event_raw(0, logical.x, logical.y);
            }
            WindowEvent::MouseInput { state, button: winit::event::MouseButton::Left, .. } => {
                let kind = if state == winit::event::ElementState::Pressed { 1 } else { 2 };
                push_mouse_event_raw(kind, self.cursor_pos.0, self.cursor_pos.1);
            }
            // Track hovered file position so we know which deck the user is aiming at
            WindowEvent::HoveredFile(_) => {
                let win_width = self.window.as_ref()
                    .map(|w| {
                        let scale = w.scale_factor();
                        w.inner_size().to_logical::<f32>(scale).width
                    })
                    .unwrap_or(1100.0);
                self.hovered_deck = if self.cursor_pos.0 < win_width * 0.5 { 1 } else { 2 };
                eprintln!("[D&D] hover x={:.0} width={:.0} -> deck {}", self.cursor_pos.0, win_width, self.hovered_deck);
            }
            // winit drag-and-drop (fires when SujayMouseView doesn't handle the drop)
            WindowEvent::DroppedFile(path) => {
                eprintln!("[D&D] DroppedFile {:?} -> deck {}", path, self.hovered_deck);
                spawn_decode(self.hovered_deck, path, self.decode_tx.clone());
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drain UI actions and dispatch to engine
        if let Some(ref engine) = self.engine {
            let engine = Arc::clone(engine);
            let tx = self.decode_tx.clone();
            for action in poll_actions_raw() {
                dispatch_action(&engine, action, &tx);
            }
        }

        // Push latest engine state into the UI renderer
        if let Ok(mut guard) = self.last_state.lock() {
            if let Some(state) = guard.take() {
                let cv = engine_state_to_console_visual(&state);
                set_console_state_raw(cv);

                let sr = state.sample_rate as f32;
                if let Some(pos) = state.deck_a_position {
                    set_deck_progress_raw(1, pos as f32, state.deck_a_total_frames.unwrap_or(0.0) as f32, sr);
                }
                if let Some(pos) = state.deck_b_position {
                    set_deck_progress_raw(2, pos as f32, state.deck_b_total_frames.unwrap_or(0.0) as f32, sr);
                }
            }
        }

        // Drain completed background decodes and load into engine on main thread
        if let Some(ref engine) = self.engine {
            while let Ok(ready) = self.decode_rx.try_recv() {
                eprintln!("[D&D] Loading deck {} title={:?} bpm={:?}", ready.deck, ready.title, ready.bpm);
                let _ = engine.load_track(
                    ready.deck as u32,
                    ready.pcm,
                    ready.bpm,
                    Some(ready.title),
                );
                let sr = engine.sample_rate as f32;
                let total_frames = ready.total_frames;
                sujay_ui::set_waveform_raw(ready.deck as u32, ready.waveform);
                // Debug: verify beat units match frame units (must clone before move)
                {
                    let first_beat = ready.beats.first().copied();
                    let last_beat  = ready.beats.last().copied();
                    if let (Some(fb), Some(lb)) = (first_beat, last_beat) {
                        eprintln!(
                            "[DEBUG] deck={} sr={} total_frames={} beats[0]={:.0} beats[-1]={:.0} \
                             beat[0]_sec={:.2} beat[-1]_sec={:.2}",
                            ready.deck, sr, total_frames, fb, lb, fb / sr, lb / sr,
                        );
                    }
                }
                sujay_ui::set_deck_markers_raw(
                    ready.deck as u32,
                    ready.beats,
                    ready.intro,
                    ready.outro,
                );
                // Initialise progress so audio_sample_rate is set before first play
                sujay_ui::set_deck_progress_raw(ready.deck as u32, 0.0, total_frames, sr);
            }
        }

        // Request redraw every frame (renderer manages its own vsync)
        if let Some(ref w) = self.window { w.request_redraw(); }
    }
}

// ── Action dispatch ───────────────────────────────────────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn dispatch_action(engine: &Arc<AudioEngineCore>, action: sujay_ui::UiAction, decode_tx: &Sender<DecodeReady>) {
    use sujay_ui::UiAction;
    match action {
        UiAction::Play(deck)              => { let _ = engine.play(deck as u32); }
        UiAction::Stop(deck)              => { let _ = engine.stop(deck as u32); }
        UiAction::SetCrossfader(v)        => { let _ = engine.set_crossfader_position(v as f64); }
        UiAction::SetMasterTempo(v)       => { let _ = engine.set_master_tempo(v as f64); }
        UiAction::SetDeckGain(deck, v)    => { let _ = engine.set_deck_gain(deck as u32, v as f64); }
        UiAction::SetCue(deck, enabled)   => { let _ = engine.set_deck_cue_enabled(deck as u32, enabled); }
        UiAction::SetEq(deck, band, kill) => { let _ = engine.set_eq_cut(deck as u32, band, kill); }
        UiAction::Seek(deck, pos)         => { let _ = engine.seek(deck as u32, pos as f64); }
        UiAction::ToggleLoop(deck, beats) => {
            if beats <= 0.0 {
                // Clear loop
                let _ = engine.clear_loop(deck as u32);
            } else if let Some((beat_grid, current_pos)) = sujay_ui::get_deck_beat_info_raw(deck as u32) {
                // Snap start to the nearest beat at or before current position
                let start_beat_idx = beat_grid.partition_point(|&b| b <= current_pos)
                    .saturating_sub(1);
                let start_frames = beat_grid.get(start_beat_idx).copied().unwrap_or(current_pos);

                // Compute end: walk `beats` steps forward in the beat grid
                let beats_whole = beats.floor() as usize;
                let beats_frac  = beats - beats.floor();
                let end_frames = if beats_frac < 0.001 {
                    // Integer beats — use beat grid directly
                    let end_idx = start_beat_idx + beats_whole;
                    if end_idx < beat_grid.len() {
                        beat_grid[end_idx]
                    } else {
                        // Past end of grid — extrapolate from last available interval
                        let beat_interval = if beat_grid.len() >= 2 {
                            beat_grid[beat_grid.len()-1] - beat_grid[beat_grid.len()-2]
                        } else {
                            engine.sample_rate as f32 * 60.0 / 120.0 // fallback 120 BPM
                        };
                        start_frames + beat_interval * beats_whole as f32
                    }
                } else {
                    // Fractional beats — use one-beat duration from grid
                    let beat_interval = if start_beat_idx + 1 < beat_grid.len() {
                        beat_grid[start_beat_idx + 1] - start_frames
                    } else if beat_grid.len() >= 2 {
                        beat_grid[beat_grid.len()-1] - beat_grid[beat_grid.len()-2]
                    } else {
                        engine.sample_rate as f32 * 60.0 / 120.0
                    };
                    start_frames + beat_interval * beats
                };

                let sr = engine.sample_rate as f64;
                let _ = engine.set_beat_loop(
                    deck as u32,
                    start_frames as f64 / sr,
                    end_frames   as f64 / sr,
                );
            } else {
                // No beat grid — fall back to simple interval from current position
                let _ = engine.toggle_beat_loop(deck as u32, beats);
            }
        }
        UiAction::LoadFile(deck, path)    => {
            spawn_decode(deck, PathBuf::from(path), decode_tx.clone());
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn dispatch_action(_engine: &Arc<AudioEngineCore>, _action: std::convert::Infallible, _tx: &Sender<DecodeReady>) {}

/// Decode `path` on a background thread and send the result via `tx`.
fn spawn_decode(deck: u8, path: PathBuf, tx: Sender<DecodeReady>) {
    std::thread::spawn(move || {
        let path_str = path.to_string_lossy().to_string();
        eprintln!("[D&D] Decoding {} for deck {}", path_str, deck);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sujay_audio::decoder::decode_audio(path_str.clone(), 44100, 2)
        }));
        match res {
            Err(e) => {
                let msg = e.downcast_ref::<&str>().copied()
                    .or_else(|| e.downcast_ref::<String>().map(|s| s.as_str()))
                    .unwrap_or("(unknown panic)");
                eprintln!("[D&D] Decode PANICKED: {}", msg);
            }
            Ok(Err(e)) => eprintln!("[D&D] Decode failed: {}", e),
            Ok(Ok(result)) => {
                let sr = result.sample_rate as f32;
                let bpm = result.bpm.map(|b| b as f32);
                // 44100 Hz → ~200 Hz (step=220): 5min track ≈ 60k points.
                // The zoom view shows an 8-sec window, giving ~1600 points of detail.
                // Use peak amplitude over each chunk so transients are visible.
                let step = (result.sample_rate as usize / 200).max(1);
                let waveform: Vec<f32> = result.mono.chunks(step)
                    .map(|chunk| chunk.iter().map(|&s| s.abs()).fold(0.0f32, f32::max))
                    .collect();
                let title = path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                // Convert beat/intro/outro from seconds → audio frames
                let (beats, intro, outro) = if let Some(ref st) = result.structure {
                    let beats = st.beats.iter().map(|&s| s as f32 * sr).collect();
                    let intro = Some(st.intro.end as f32 * sr);
                    let outro = Some(st.outro.start as f32 * sr);
                    (beats, intro, outro)
                } else {
                    (vec![], None, None)
                };
                eprintln!("[D&D] Decode done deck={} bpm={:?} beats={} title={:?}", deck, bpm, beats.len(), title);
                let total_frames = (result.pcm.len() / 2) as f32; // stereo → mono frames
                let _ = tx.send(DecodeReady { deck, pcm: result.pcm, waveform, bpm, title, beats, intro, outro, total_frames });
            }
        }
    });
}

// ── Engine state → UI visual state mapping ───────────────────────────────────

fn engine_state_to_console_visual(
    s: &sujay_audio::engine_core::EngineStateUpdate,
) -> sujay_ui::ui_state::ConsoleVisualState {
    use sujay_ui::ui_state::{ConsoleVisualState, DeckConsoleVisualState};

    fn fmt_time(frames: f64, sr: f64) -> String {
        if sr == 0.0 { return "0:00".into(); }
        let secs = (frames / sr) as u64;
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    let sr = s.sample_rate;

    // Compute loop_beats from loop length and track BPM so the correct button
    // appears highlighted.  Falls back to 0.0 (no active button) if unknown.
    let calc_loop_beats = |loop_enabled: bool, start: f64, end: f64, bpm: Option<f64>| -> f32 {
        if !loop_enabled || bpm.is_none() || bpm == Some(0.0) { return 0.0; }
        let beat_interval = sr * 60.0 / bpm.unwrap();
        let beats = (end - start) / beat_interval;
        // Round to nearest standard value
        let standards = [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
        standards.iter().copied().min_by(|a, b| {
            (a - beats as f32).abs().partial_cmp(&(b - beats as f32).abs()).unwrap()
        }).unwrap_or(0.0)
    };

    let deck_a = DeckConsoleVisualState {
        title:        s.deck_a_track_id.clone().unwrap_or_else(|| "---".into()),
        time_text:    s.deck_a_position.map(|p| fmt_time(p, sr)).unwrap_or_else(|| "0:00".into()),
        bpm_text:     s.deck_a_bpm.map(|b| format!("{:.1}", b)).unwrap_or_else(|| "--.-".into()),
        bpm:          s.deck_a_bpm.unwrap_or(s.master_tempo) as f32,
        playing:      s.deck_a_playing,
        loop_enabled: s.deck_a_loop.enabled,
        loop_beats:   calc_loop_beats(s.deck_a_loop.enabled, s.deck_a_loop.start, s.deck_a_loop.end, s.deck_a_bpm),
        loop_start:   s.deck_a_loop.start as f32,
        loop_end:     s.deck_a_loop.end as f32,
        cue_enabled:  s.deck_a_cue_enabled,
        eq_low:       s.deck_a_eq_cut.low,
        eq_mid:       s.deck_a_eq_cut.mid,
        eq_high:      s.deck_a_eq_cut.high,
        gain:         s.deck_a_gain as f32,
        peak:         s.deck_a_peak as f32,
    };
    let deck_b = DeckConsoleVisualState {
        title:        s.deck_b_track_id.clone().unwrap_or_else(|| "---".into()),
        time_text:    s.deck_b_position.map(|p| fmt_time(p, sr)).unwrap_or_else(|| "0:00".into()),
        bpm_text:     s.deck_b_bpm.map(|b| format!("{:.1}", b)).unwrap_or_else(|| "--.-".into()),
        bpm:          s.deck_b_bpm.unwrap_or(s.master_tempo) as f32,
        playing:      s.deck_b_playing,
        loop_enabled: s.deck_b_loop.enabled,
        loop_beats:   calc_loop_beats(s.deck_b_loop.enabled, s.deck_b_loop.start, s.deck_b_loop.end, s.deck_b_bpm),
        loop_start:   s.deck_b_loop.start as f32,
        loop_end:     s.deck_b_loop.end as f32,
        cue_enabled:  s.deck_b_cue_enabled,
        eq_low:       s.deck_b_eq_cut.low,
        eq_mid:       s.deck_b_eq_cut.mid,
        eq_high:      s.deck_b_eq_cut.high,
        gain:         s.deck_b_gain as f32,
        peak:         s.deck_b_peak as f32,
    };
    ConsoleVisualState {
        deck_a,
        deck_b,
        master_tempo: s.master_tempo as f32,
        crossfader:   s.crossfader_position as f32,
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Sujay starting (Phase 5 Rust-native)");

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = SujayApp::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
