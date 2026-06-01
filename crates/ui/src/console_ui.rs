//! Platform-independent egui DJ-console UI.
//!
//! All immediate-mode drawing, shared visual state, the public state-setter
//! API, and the wgpu + egui_wgpu render loop live here. The per-platform
//! renderers (`renderer.rs` on macOS, `renderer_windows.rs` on Windows) only
//! provide surface creation and host-window attachment, then hand a ready
//! `wgpu` surface to [`run_egui_render_loop`].

use crate::ui_state::{ConsoleVisualState, DeckConsoleVisualState, PreferencesState};
use egui_wgpu::wgpu;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DSEG7_FONT: &[u8] =
    include_bytes!("../assets/fonts/DSEG7Classic-Regular.ttf");
const PIXEL_FONT: &[u8] =
    include_bytes!("../assets/fonts/PixelMplus12-Regular.ttf");

// ── Color palette (matches React CSS) ──────────────────────────────────────
const BG_DARK: egui::Color32          = egui::Color32::from_rgb(26, 26, 26);
#[allow(dead_code)]
const PANEL_BG: egui::Color32         = egui::Color32::from_rgb(42, 42, 42);
#[allow(dead_code)]
const PANEL_BG_DARK: egui::Color32    = egui::Color32::from_rgb(26, 26, 26);
const BORDER_DIM: egui::Color32       = egui::Color32::from_rgb(68, 68, 68);
const BORDER_MED: egui::Color32       = egui::Color32::from_rgb(85, 85, 85);
const CYAN: egui::Color32             = egui::Color32::from_rgb(0, 212, 255);
const ORANGE: egui::Color32           = egui::Color32::from_rgb(255, 69, 0);
const ORANGE_SEC: egui::Color32       = egui::Color32::from_rgb(255, 107, 53);
const GREEN: egui::Color32            = egui::Color32::from_rgb(0, 204, 102);
const TEXT_PRIMARY: egui::Color32     = egui::Color32::from_rgb(224, 224, 224);
const TEXT_DIM: egui::Color32         = egui::Color32::from_rgb(208, 208, 208);
const WAVEFORM_PLAYED: egui::Color32  = egui::Color32::from_rgb(74, 158, 255);
const WAVEFORM_UNPLAYED: egui::Color32= egui::Color32::from_rgb(221, 221, 221);
const WAVEFORM_EMPTY: egui::Color32   = egui::Color32::from_rgb(51, 51, 51);
const METER_GREEN: egui::Color32      = egui::Color32::from_rgb(0, 255, 0);
const METER_ORANGE: egui::Color32     = egui::Color32::from_rgb(255, 136, 0);
const METER_RED: egui::Color32        = egui::Color32::from_rgb(255, 0, 0);
const BUTTON_BG: egui::Color32        = egui::Color32::from_rgb(51, 51, 51);

// ── State types ────────────────────────────────────────────────────────────
#[derive(Clone, Default)]
pub struct DeckVisualState {
    pub progress: f32,           // audio frame index
    pub total_frames: f32,       // total audio frames (pcm_len / channels)
    pub audio_sample_rate: f32,  // audio sample rate in Hz (e.g. 44100)
    pub beats: Vec<f32>,         // audio frame indices
    pub intro: Option<f32>,      // audio frame index
    pub outro: Option<f32>,      // audio frame index
    pub peak_hold: f32,
}

// ── UI Action (egui → JS/audio worker) ─────────────────────────────────────
#[derive(Clone, Debug)]
pub enum UiAction {
    Play(u8),           // deck 1 or 2
    Stop(u8),
    SetCrossfader(f32),
    SetMasterTempo(f32),
    SetCue(u8, bool),   // deck, enabled
    SetEq(u8, &'static str, bool), // deck, band, enabled
    ToggleLoop(u8, f32),  // deck, beats (0 = clear)
    Seek(u8, f32),      // deck, position 0..1
    SetDeckGain(u8, f32), // deck, gain 0..1
    LoadFile(u8, String), // deck, absolute local file path
    SetMicEnabled(bool),
    StartRecording,
    StopRecording,
    OpenPreferences,
    SavePreferences(PreferencesState),
}

// ── Mouse input events (host window → egui) ────────────────────────────────
#[derive(Clone)]
pub(crate) enum MouseEvent {
    Moved(f32, f32),
    Pressed(f32, f32),
    Released(f32, f32),
}

// ── Global shared state ────────────────────────────────────────────────────
static WAVEFORMS: Mutex<[Vec<f32>; 2]> = Mutex::new([Vec::new(), Vec::new()]);

pub static DECK_VISUALS: Mutex<[DeckVisualState; 2]> = Mutex::new([
    DeckVisualState { progress: 0.0, total_frames: 0.0, audio_sample_rate: 0.0, beats: Vec::new(), intro: None, outro: None, peak_hold: 0.0 },
    DeckVisualState { progress: 0.0, total_frames: 0.0, audio_sample_rate: 0.0, beats: Vec::new(), intro: None, outro: None, peak_hold: 0.0 },
]);
static CONSOLE_VISUAL: Mutex<ConsoleVisualState> = Mutex::new(ConsoleVisualState {
    titlebar: crate::ui_state::TitlebarState {
        time_text: String::new(),
        cpu_percent: 0.0,
        mem_mb: 0,
        mic_available: false,
        mic_enabled: false,
        mic_peak: 0.0,
        is_recording: false,
        rec_elapsed_secs: 0,
    },
    deck_a: DeckConsoleVisualState {
        title: String::new(), time_text: String::new(), bpm_text: String::new(), bpm: 0.0,
        playing: false, loop_enabled: false, loop_beats: 0.0, loop_start: 0.0, loop_end: 0.0, cue_enabled: false,
        eq_low: false, eq_mid: false, eq_high: false, gain: 1.0, peak: 0.0,
    },
    deck_b: DeckConsoleVisualState {
        title: String::new(), time_text: String::new(), bpm_text: String::new(), bpm: 0.0,
        playing: false, loop_enabled: false, loop_beats: 0.0, loop_start: 0.0, loop_end: 0.0, cue_enabled: false,
        eq_low: false, eq_mid: false, eq_high: false, gain: 1.0, peak: 0.0,
    },
    master_tempo: 130.0,
    crossfader: 0.5,
});
static PREFS_VISUAL: LazyLock<Mutex<PreferencesState>> =
    LazyLock::new(|| Mutex::new(PreferencesState::default()));
static PREFS_DRAFT: LazyLock<Mutex<PreferencesState>> =
    LazyLock::new(|| Mutex::new(PreferencesState::default()));
static PREFS_OPEN: AtomicBool = AtomicBool::new(false);
// 0 = Audio, 1 = Recording, 2 = OSC
static PREFS_TAB: AtomicU8 = AtomicU8::new(0);
#[allow(dead_code)]
static WAVEFORM_VERSIONS: [AtomicU64; 2] = [AtomicU64::new(1), AtomicU64::new(1)];
pub(crate) static MOUSE_EVENTS: Mutex<Vec<MouseEvent>> = Mutex::new(Vec::new());
static UI_ACTIONS: Mutex<Vec<UiAction>> = Mutex::new(Vec::new());
pub(crate) static NEEDS_REPAINT: AtomicBool = AtomicBool::new(true);

/// RGBA image data for deck artwork. (width, height, pixels)
static DECK_ARTWORK: Mutex<[Option<(u32, u32, Vec<u8>)>; 2]> = Mutex::new([None, None]);
/// Monotonically increasing version counter per deck – bumped on each set_artwork call.
static ARTWORK_VERSIONS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

// ═══════════════════════════════════════════════════════════════════════════
// Font & style setup
// ═══════════════════════════════════════════════════════════════════════════

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "pixel".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(PIXEL_FONT)),
    );
    fonts.font_data.insert(
        "dseg7".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(DSEG7_FONT)),
    );

    // Pixel font as default proportional
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "pixel".to_owned());

    // DSEG7 as named family for tempo display
    fonts.families.insert(
        egui::FontFamily::Name("dseg7".into()),
        vec!["dseg7".to_owned()],
    );

    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG_DARK;
    visuals.window_fill = BG_DARK;
    visuals.override_text_color = Some(TEXT_PRIMARY);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(4.0, 4.0);
    ctx.set_style(style);
}

// ═══════════════════════════════════════════════════════════════════════════
// Drawing helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Draw a horizontal gradient rectangle using a mesh.
fn paint_gradient_h(
    painter: &egui::Painter,
    rect: egui::Rect,
    left: egui::Color32,
    right: egui::Color32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Draw a 135-degree diagonal gradient (top-left → bottom-right).
fn paint_gradient_135(
    painter: &egui::Painter,
    rect: egui::Rect,
    top_left: egui::Color32,
    bottom_right: egui::Color32,
    corner_radius: f32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    // For a 135° gradient, top-right and bottom-left get the average color
    let mix = |a: egui::Color32, b: egui::Color32| -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(
            ((a.r() as u16 + b.r() as u16) / 2) as u8,
            ((a.g() as u16 + b.g() as u16) / 2) as u8,
            ((a.b() as u16 + b.b() as u16) / 2) as u8,
            ((a.a() as u16 + b.a() as u16) / 2) as u8,
        )
    };
    let mid = mix(top_left, bottom_right);
    let _ = corner_radius; // egui Mesh doesn't easily support rounded corners
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top_left);
    mesh.colored_vertex(rect.right_top(), mid);
    mesh.colored_vertex(rect.right_bottom(), bottom_right);
    mesh.colored_vertex(rect.left_bottom(), mid);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Paint a glow (box-shadow) effect around a rect.
fn paint_glow(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    spread: f32,
) {
    let steps = 4;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let expand = spread * t;
        let alpha = ((1.0 - t) * color.a() as f32 * 0.3) as u8;
        let c = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        let outer = rect.expand(expand);
        painter.rect_stroke(outer, 4.0, egui::Stroke::new(1.0, c), egui::StrokeKind::Outside);
    }
}

fn peak_to_db(peak: f32) -> f32 {
    if peak <= 0.0 {
        return f32::NEG_INFINITY;
    }
    let dbfs = 20.0 * peak.log10();
    (dbfs + 8.0).min(13.0) // +8 dB offset (Pioneer calibration)
}

pub(crate) fn push_action(action: UiAction) {
    UI_ACTIONS.lock().unwrap().push(action);
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
}

/// Open the egui preferences modal. Used on platforms without a native
/// settings dialog (e.g. Windows); macOS opens its NSPanel dialog instead.
pub fn open_preferences() {
    PREFS_OPEN.store(true, Ordering::Relaxed);
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
}

pub fn drain_actions() -> Vec<UiAction> {
    let actions = std::mem::take(&mut *UI_ACTIONS.lock().unwrap());
    actions
}

// ═══════════════════════════════════════════════════════════════════════════
// UI Components
// ═══════════════════════════════════════════════════════════════════════════

// ── Zoom waveform (8-second viewport) ──────────────────────────────────────

fn draw_zoom_waveform(ui: &mut egui::Ui, samples: &[f32], visual: &DeckVisualState, deck_state: &DeckConsoleVisualState, master_tempo: f32) {
    let width = ui.available_width();
    let height = 56.0_f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();

    painter.rect_filled(rect, 0.0, WAVEFORM_EMPTY);

    let total_frames = visual.total_frames;
    if samples.is_empty() || total_frames <= 0.0 || visual.audio_sample_rate <= 0.0 {
        return;
    }

    // All positions are audio frame indices
    let current_pos = visual.progress;

    // Scale the visible window by the playback rate so that the view always
    // represents ~8 real-time seconds regardless of master tempo.
    // rate = master_tempo / track_bpm  (>1 = faster, consuming more input frames/sec)
    let rate = if deck_state.bpm > 0.0 && master_tempo > 0.0 {
        (master_tempo / deck_state.bpm).clamp(0.5, 2.0)
    } else {
        1.0
    };
    let visible_seconds = 8.0_f32;
    let visible_frames = (visible_seconds * visual.audio_sample_rate * rate).min(total_frames);
    let playback_offset = 0.3_f32;

    let mut view_start = current_pos - visible_frames * playback_offset;
    let mut view_end = view_start + visible_frames;
    if view_start < 0.0 {
        view_start = 0.0;
        view_end = visible_frames;
    }
    if view_end > total_frames {
        view_end = total_frames;
        view_start = (total_frames - visible_frames).max(0.0);
    }
    let view_span = (view_end - view_start).max(1.0);

    // audio frame index → pixel x
    let to_x = |pos: f32| -> f32 {
        rect.min.x + ((pos - view_start) / view_span).clamp(0.0, 1.0) * width
    };

    // Render waveform by iterating over screen pixel columns and looking up the
    // corresponding waveform sample for each column.  This guarantees that
    // waveform peaks and beat markers (both in audio-frame coordinates) are drawn
    // at the same pixel — previous code iterated over waveform samples and spread
    // them evenly across pixels, covering only ~67 % of view_span and causing
    // progressive drift between markers and waveform.
    let wf_len = samples.len() as f32;
    let frames_per_wf_sample = total_frames / wf_len; // audio frames per downsampled sample
    // How many waveform samples does one pixel column span?
    let wf_samples_per_pixel = (view_span / width) / frames_per_wf_sample;

    let progress_x = to_x(current_pos);
    let cy = rect.center().y;
    let bar_w = 1.0_f32;

    let pixel_count = width as usize;
    for px in 0..pixel_count {
        // Audio frame at the left edge of this pixel
        let frame_left  = view_start + (px as f32 / width) * view_span;
        let frame_right = view_start + ((px + 1) as f32 / width) * view_span;

        // Corresponding waveform sample indices
        let wf_lo = ((frame_left  / total_frames) * wf_len).floor().max(0.0) as usize;
        let wf_hi = ((frame_right / total_frames) * wf_len).ceil()
                        .min(wf_len) as usize;
        let wf_lo = wf_lo.min(samples.len().saturating_sub(1));
        let wf_hi = wf_hi.max(wf_lo + 1).min(samples.len());

        let mut max_amp: f32 = 0.0;
        for j in wf_lo..wf_hi {
            max_amp = max_amp.max(samples[j].abs());
        }

        let x = rect.min.x + px as f32;
        let bh = max_amp * height * 0.45;
        let color = if x < progress_x { WAVEFORM_PLAYED } else { WAVEFORM_UNPLAYED };
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(x + bar_w * 0.5, cy),
                egui::vec2(bar_w, bh * 2.0),
            ),
            0.0, color,
        );
    }
    let _ = wf_samples_per_pixel; // used above for documentation

    // Beat markers (audio frame indices)
    let beat_color = egui::Color32::from_rgba_unmultiplied(255, 100, 100, 204);
    for &beat in visual.beats.iter() {
        if beat >= view_start && beat <= view_end {
            painter.vline(to_x(beat), rect.y_range(), egui::Stroke::new(1.0, beat_color));
        }
    }

    // Intro marker (audio frame index)
    if let Some(intro) = visual.intro {
        if intro >= view_start && intro <= view_end {
            painter.vline(to_x(intro), rect.y_range(), egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(100, 255, 100, 204)));
        }
    }

    // Outro marker (audio frame index)
    if let Some(outro) = visual.outro {
        if outro >= view_start && outro <= view_end {
            painter.vline(to_x(outro), rect.y_range(), egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 255, 100, 204)));
        }
    }

    // Loop region (audio frame indices)
    if deck_state.loop_enabled && deck_state.loop_start < deck_state.loop_end {
        let lx1 = to_x(deck_state.loop_start);
        let lx2 = to_x(deck_state.loop_end);
        if lx2 > lx1 {
            let loop_rect = egui::Rect::from_min_max(
                egui::pos2(lx1, rect.min.y),
                egui::pos2(lx2, rect.max.y),
            );
            painter.rect_filled(loop_rect, 0.0, egui::Color32::from_rgba_unmultiplied(0, 204, 102, 40));
            painter.vline(lx1, rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 204, 102, 180)));
            painter.vline(lx2, rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 204, 102, 180)));
        }
    }

    // Playhead
    painter.vline(progress_x, rect.y_range(), egui::Stroke::new(2.0, egui::Color32::WHITE));
}

// ── Full waveform ──────────────────────────────────────────────────────────

fn draw_full_waveform(ui: &mut egui::Ui, deck: u8, samples: &[f32], visual: &DeckVisualState, deck_state: &DeckConsoleVisualState, height: f32) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let painter = ui.painter();

    // Seek on click
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let seek_pos = ((pos.x - rect.min.x) / width).clamp(0.0, 1.0);
            push_action(UiAction::Seek(deck, seek_pos));
        }
    }

    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(26, 26, 26));

    let total_frames = visual.total_frames;
    if samples.is_empty() || total_frames <= 0.0 {
        return;
    }

    // audio frame index → pixel x (full track maps to full width)
    let to_x = |pos: f32| -> f32 {
        rect.min.x + (pos / total_frames).clamp(0.0, 1.0) * width
    };

    let bar_count = (width as usize).min(512).max(1);
    let step = (samples.len() as f32 / bar_count as f32).max(1.0);
    let bar_w = width / bar_count as f32;
    let cy = rect.center().y;
    let progress_x = to_x(visual.progress);

    for i in 0..bar_count {
        let idx_start = ((i as f32) * step).floor() as usize;
        let idx_end = (((i + 1) as f32) * step).floor().min(samples.len() as f32) as usize;
        let mut max_amp: f32 = 0.0;
        for j in idx_start..idx_end.min(samples.len()) {
            max_amp = max_amp.max(samples[j].abs());
        }
        let x = rect.min.x + i as f32 * bar_w;
        let bh = max_amp * (height * 0.5) * 0.9;
        let color = if x < progress_x { WAVEFORM_PLAYED } else { WAVEFORM_UNPLAYED };
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(x + bar_w * 0.5, cy),
                egui::vec2((bar_w - 1.0).max(1.0), bh * 2.0),
            ),
            0.0, color,
        );
    }

    // Intro marker (sample index)
    if let Some(intro) = visual.intro {
        painter.vline(to_x(intro), rect.y_range(), egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(100, 255, 100, 204)));
    }

    // Outro marker (sample index)
    if let Some(outro) = visual.outro {
        painter.vline(to_x(outro), rect.y_range(), egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 255, 100, 204)));
    }

    // Loop region (sample indices)
    if deck_state.loop_enabled && deck_state.loop_start < deck_state.loop_end {
        let lx1 = to_x(deck_state.loop_start);
        let lx2 = to_x(deck_state.loop_end);
        if lx2 > lx1 {
            let loop_rect = egui::Rect::from_min_max(
                egui::pos2(lx1, rect.min.y),
                egui::pos2(lx2, rect.max.y),
            );
            painter.rect_filled(loop_rect, 0.0, egui::Color32::from_rgba_unmultiplied(0, 204, 102, 40));
            painter.vline(lx1, rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 204, 102, 180)));
            painter.vline(lx2, rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 204, 102, 180)));
        }
    }

    // Playhead
    painter.vline(progress_x, rect.y_range(), egui::Stroke::new(2.0, egui::Color32::WHITE));
}

// ── Level meter (Pioneer 15-segment LED) ───────────────────────────────────

fn draw_level_meter(ui: &mut egui::Ui, peak: f32, peak_hold: f32, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();

    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(15, 15, 15));

    let min_db: f32 = -24.0;
    let max_db: f32 = 13.0;
    let num_segments: usize = 15;
    let seg_h = height / num_segments as f32;
    let current_db = peak_to_db(peak);
    let hold_db = peak_to_db(peak_hold);

    for i in 0..num_segments {
        let seg_db = min_db + (i as f32 / (num_segments - 1) as f32) * (max_db - min_db);

        let color = if i >= 13 {
            METER_RED
        } else if i >= 9 {
            METER_ORANGE
        } else {
            METER_GREEN
        };

        let y = rect.max.y - ((i + 1) as f32 / num_segments as f32) * height;
        let seg_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, y),
            egui::vec2(width, seg_h - 1.0),
        );

        if current_db >= seg_db {
            painter.rect_filled(seg_rect, 0.0, color);
        } else if hold_db >= seg_db && hold_db < seg_db + (max_db - min_db) / (num_segments - 1) as f32 {
            // Peak hold indicator: single bright segment
            painter.rect_filled(seg_rect, 0.0, color);
        }
    }
}

// ── Volume slider ──────────────────────────────────────────────────────────

fn draw_volume_slider(ui: &mut egui::Ui, deck: u8, gain: f32, width: f32, height: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let painter = ui.painter();

    // Handle drag/click on volume slider
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let new_gain = 1.0 - ((pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
            push_action(UiAction::SetDeckGain(deck, new_gain));
        }
    }

    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(26, 26, 26));
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(85, 85, 85)), egui::StrokeKind::Outside);

    let fill_h = gain.clamp(0.0, 1.0) * rect.height();
    if fill_h > 1.0 {
        let fill_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + 1.0, rect.max.y - fill_h),
            egui::pos2(rect.max.x - 1.0, rect.max.y - 1.0),
        );
        painter.rect_filled(fill_rect, 1.0, CYAN.linear_multiply(0.6));
    }

    let handle_y = rect.max.y - gain.clamp(0.0, 1.0) * rect.height();
    let handle_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, handle_y),
        egui::vec2(rect.width(), 8.0),
    );
    paint_gradient_135(painter, handle_rect, egui::Color32::from_rgb(68, 68, 68), egui::Color32::from_rgb(42, 42, 42), 1.0);
    painter.rect_stroke(handle_rect, 1.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(102, 102, 102)), egui::StrokeKind::Outside);
}

// ── EQ kill buttons ────────────────────────────────────────────────────────

fn draw_eq_kills_column(ui: &mut egui::Ui, deck: u8, high: bool, mid: bool, low: bool) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 3.0);
        draw_eq_kill_button(ui, deck, "H", "high", high);
        draw_eq_kill_button(ui, deck, "M", "mid", mid);
        draw_eq_kill_button(ui, deck, "L", "low", low);
    });
}

fn draw_eq_kill_button(ui: &mut egui::Ui, deck: u8, label: &str, band: &'static str, active: bool) {
    let size = egui::vec2(32.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();

    if resp.clicked() {
        push_action(UiAction::SetEq(deck, band, !active));
    }

    let hover = resp.hovered();
    let draw_rect = if resp.is_pointer_button_down_on() { rect.translate(egui::vec2(0.0, 1.0)) } else { rect };
    if active {
        paint_gradient_135(painter, draw_rect, ORANGE, egui::Color32::from_rgb(204, 55, 0), 3.0);
        painter.rect_stroke(draw_rect, 3.0, egui::Stroke::new(1.0, ORANGE), egui::StrokeKind::Outside);
    } else {
        let bg = if hover { egui::Color32::from_rgb(62, 62, 62) } else { BUTTON_BG };
        paint_gradient_135(painter, draw_rect, bg, egui::Color32::from_rgb(31, 31, 31), 3.0);
        painter.rect_stroke(draw_rect, 3.0, egui::Stroke::new(1.0, BORDER_MED), egui::StrokeKind::Outside);
    }
    let text_color = if active { egui::Color32::WHITE } else { TEXT_DIM };
    painter.text(
        draw_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        text_color,
    );
}

// ── Cue button ─────────────────────────────────────────────────────────────

fn draw_cue_button(ui: &mut egui::Ui, deck: u8, enabled: bool) {
    let size = egui::vec2(32.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();

    if resp.clicked() {
        push_action(UiAction::SetCue(deck, !enabled));
    }

    let hover = resp.hovered();
    let draw_rect = if resp.is_pointer_button_down_on() { rect.translate(egui::vec2(0.0, 1.0)) } else { rect };
    if enabled {
        paint_gradient_135(painter, draw_rect, egui::Color32::from_rgb(32, 42, 58), egui::Color32::from_rgb(15, 22, 32), 4.0);
        painter.rect_stroke(draw_rect, 4.0, egui::Stroke::new(1.0, CYAN), egui::StrokeKind::Outside);
        paint_glow(painter, draw_rect, CYAN, 6.0);
    } else {
        let bg = if hover { egui::Color32::from_rgb(62, 62, 62) } else { BUTTON_BG };
        paint_gradient_135(painter, draw_rect, bg, egui::Color32::from_rgb(31, 31, 31), 4.0);
        painter.rect_stroke(draw_rect, 4.0, egui::Stroke::new(1.0, BORDER_MED), egui::StrokeKind::Outside);
    }
    let fg = if enabled { CYAN } else { TEXT_DIM };
    // Draw headphone icon (simplified)
    let cx = rect.center().x;
    let cy = rect.center().y;
    // Arc (headband)
    let arc_r = 6.0;
    let segments = 12;
    for i in 0..segments {
        let a1 = std::f32::consts::PI + (i as f32 / segments as f32) * std::f32::consts::PI;
        let a2 = std::f32::consts::PI + ((i + 1) as f32 / segments as f32) * std::f32::consts::PI;
        let p1 = egui::pos2(cx + arc_r * a1.cos(), cy - 2.0 + arc_r * a1.sin());
        let p2 = egui::pos2(cx + arc_r * a2.cos(), cy - 2.0 + arc_r * a2.sin());
        painter.line_segment([p1, p2], egui::Stroke::new(1.5, fg));
    }
    // Left ear cup
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(cx - arc_r - 1.5, cy - 2.0), egui::vec2(3.0, 6.0)),
        1.0,
        fg,
    );
    // Right ear cup
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(cx + arc_r - 1.5, cy - 2.0), egui::vec2(3.0, 6.0)),
        1.0,
        fg,
    );
}

// ── Loop buttons ───────────────────────────────────────────────────────────

fn draw_loop_buttons(ui: &mut egui::Ui, deck: u8, loop_enabled: bool, loop_beats: f32, align_right: bool) {
    let time = ui.input(|i| i.time);
    ui.vertical(|ui| {
        let row1: &[f32] = &[1.0 / 16.0, 1.0 / 8.0, 1.0 / 4.0, 1.0 / 2.0, 1.0];
        let row2: &[f32] = &[2.0, 4.0, 8.0, 16.0, 32.0];
        for row in [row1, row2] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                if align_right {
                    let btn_w = 26.0; // 24 + 2 spacing
                    let total = row.len() as f32 * btn_w;
                    let space = (ui.available_width() - total).max(0.0);
                    ui.add_space(space);
                }
                for &beats in row.iter() {
                    let active = loop_enabled && (loop_beats - beats).abs() < 0.001;
                    let label = if beats >= 1.0 {
                        format!("{}", beats as i32)
                    } else {
                        format!("1/{}", (1.0 / beats) as i32)
                    };
                    draw_loop_button(ui, deck, &label, beats, active, time);
                }
            });
        }
    });
}

fn draw_loop_button(ui: &mut egui::Ui, deck: u8, label: &str, beats: f32, active: bool, time: f64) {
    let size = egui::vec2(24.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();

    if resp.clicked() {
        push_action(UiAction::ToggleLoop(deck, if active { 0.0 } else { beats }));
    }

    let hover = resp.hovered();
    let draw_rect = if resp.is_pointer_button_down_on() { rect.translate(egui::vec2(0.0, 1.0)) } else { rect };
    if active {
        // Pulse animation: oscillate glow spread between 4..12 at 2Hz
        let pulse = 8.0 + 4.0 * (time * 2.0 * std::f64::consts::PI).sin() as f32;
        paint_gradient_135(painter, draw_rect, GREEN, egui::Color32::from_rgb(0, 153, 68), 3.0);
        painter.rect_stroke(draw_rect, 3.0, egui::Stroke::new(1.0, GREEN), egui::StrokeKind::Outside);
        paint_glow(painter, draw_rect, GREEN, pulse);
    } else {
        let bg = if hover { egui::Color32::from_rgb(62, 62, 62) } else { BUTTON_BG };
        paint_gradient_135(painter, draw_rect, bg, egui::Color32::from_rgb(31, 31, 31), 3.0);
        painter.rect_stroke(draw_rect, 3.0, egui::Stroke::new(1.0, BORDER_MED), egui::StrokeKind::Outside);
    }
    let text_color = if active { egui::Color32::WHITE } else { TEXT_DIM };
    painter.text(
        draw_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(9.0),
        text_color,
    );
}

// ── Play / Stop button ─────────────────────────────────────────────────────

fn draw_deck_number(ui: &mut egui::Ui, num: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let painter = ui.painter();
    // text-shadow: 0 0 10px rgba(0,212,255,0.6)
    let glow = egui::Color32::from_rgba_unmultiplied(0, 212, 255, 153);
    for &dx in &[-1.0_f32, 0.0, 1.0] {
        for &dy in &[-1.0_f32, 0.0, 1.0] {
            if dx == 0.0 && dy == 0.0 { continue; }
            painter.text(
                rect.center() + egui::vec2(dx, dy),
                egui::Align2::CENTER_CENTER,
                num,
                egui::FontId::proportional(18.0),
                glow,
            );
        }
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        num,
        egui::FontId::proportional(18.0),
        CYAN,
    );
}

fn draw_play_stop_button(ui: &mut egui::Ui, deck: u8, playing: bool) {
    let size = egui::vec2(32.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();

    if resp.clicked() {
        if playing {
            push_action(UiAction::Stop(deck));
        } else {
            push_action(UiAction::Play(deck));
        }
    }

    let hover = resp.hovered();
    let bg_top = if hover { egui::Color32::from_rgb(85, 85, 85) } else { egui::Color32::from_rgb(68, 68, 68) };
    let bg_bot = if hover { egui::Color32::from_rgb(55, 55, 55) } else { egui::Color32::from_rgb(42, 42, 42) };
    let draw_rect = if resp.is_pointer_button_down_on() { rect.translate(egui::vec2(0.0, 1.0)) } else { rect };
    paint_gradient_135(painter, draw_rect, bg_top, bg_bot, 4.0);
    painter.rect_stroke(draw_rect, 4.0, egui::Stroke::new(1.0, BORDER_MED), egui::StrokeKind::Outside);

    let symbol = if playing { "■" } else { "▶" };
    painter.text(
        draw_rect.center(),
        egui::Align2::CENTER_CENTER,
        symbol,
        egui::FontId::proportional(16.0),
        ORANGE_SEC,
    );
}

// ── Thumbnail placeholder ──────────────────────────────────────────────────

fn draw_thumbnail(ui: &mut egui::Ui, deck_index: usize, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();

    if let Some(tex) = get_artwork_texture(ui.ctx(), deck_index) {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, BORDER_DIM), egui::StrokeKind::Outside);
    } else {
        paint_gradient_135(painter, rect, egui::Color32::from_rgb(42, 42, 42), egui::Color32::from_rgb(26, 26, 26), 4.0);
        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, BORDER_DIM), egui::StrokeKind::Outside);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "🎵",
            egui::FontId::proportional(20.0),
            TEXT_DIM,
        );
    }
}

// ── Deck header ────────────────────────────────────────────────────────────

fn draw_deck_header(ui: &mut egui::Ui, is_left: bool, deck_num: u8, deck: &DeckConsoleVisualState) {
    let deck_index = if is_left { 0 } else { 1 };
    let row_w = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(row_w, 40.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 0.0);

            // Layout: 4 items + 3 gaps (item_spacing=10)
            // Both decks: num(18) + thumb(40) + play(32) + 3*sp(10) = 120
            let info_w = (row_w - 120.0).max(10.0);
            if is_left {
                // [1] [thumb] [info...fill...] [play/stop]
                draw_deck_number(ui, "1");
                draw_thumbnail(ui, deck_index, 40.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(info_w, 40.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(info_w);
                        draw_deck_info_text(ui, deck);
                    },
                );
                draw_play_stop_button(ui, deck_num, deck.playing);
            } else {
                // [play/stop] [info...fill...] [thumb] [2]
                draw_play_stop_button(ui, deck_num, deck.playing);
                ui.allocate_ui_with_layout(
                    egui::vec2(info_w, 40.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(info_w);
                        draw_deck_info_text(ui, deck);
                    },
                );
                draw_thumbnail(ui, deck_index, 40.0);
                draw_deck_number(ui, "2");
            }
        },
    );
}

fn draw_deck_info_text(ui: &mut egui::Ui, deck: &DeckConsoleVisualState) {
    ui.vertical(|ui| {
        let title = if deck.title.is_empty() {
            "No track loaded"
        } else {
            &deck.title
        };
        ui.add(
            egui::Label::new(egui::RichText::new(title).size(13.0).color(TEXT_PRIMARY).strong())
                .truncate(),
        );
        let time_text = if deck.time_text.is_empty() {
            "--:-- / --:--"
        } else {
            &deck.time_text
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
            ui.label(egui::RichText::new(time_text).size(11.0).color(CYAN));
            if !deck.bpm_text.is_empty() {
                ui.label(
                    egui::RichText::new(format!("• {} BPM", deck.bpm_text))
                        .size(11.0)
                        .color(ORANGE_SEC),
                );
            }
        });
    });
}

// ── Deck panel ─────────────────────────────────────────────────────────────

fn draw_deck(
    ui: &mut egui::Ui,
    is_left: bool,
    deck: &DeckConsoleVisualState,
    waveform: &[f32],
    visual: &DeckVisualState,
) {
    let has_track = !deck.title.is_empty();
    let border_color = if has_track { CYAN } else { egui::Color32::from_rgb(68, 68, 68) };

    let frame_margin = 8.0;

    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(frame_margin)
        .show(ui, |ui| {
            // Paint gradient only inside the allocated deck panel.
            // Avoid expanding outside, which can overlap the crossfader area.
            let bg_rect = ui.max_rect();
            let painter = ui.painter();
            paint_gradient_135(
                painter,
                bg_rect,
                egui::Color32::from_rgb(42, 42, 42),
                egui::Color32::from_rgb(26, 26, 26),
                0.0,
            );
            // Active deck glow
            if has_track {
                paint_glow(painter, bg_rect, CYAN, 15.0);
            }

            let deck_num: u8 = if is_left { 1 } else { 2 };
            draw_deck_header(ui, is_left, deck_num, deck);
            ui.add_space(5.0);
            draw_full_waveform(ui, deck_num, waveform, visual, deck, 50.0);
            ui.add_space(10.0);
            // Deck A (left): loop buttons align right; Deck B (right): align left
            draw_loop_buttons(ui, deck_num, deck.loop_enabled, deck.loop_beats, is_left);
        });
}

// ── Tempo section ──────────────────────────────────────────────────────────

fn draw_tempo_section(ui: &mut egui::Ui, state: &ConsoleVisualState, visuals: &[DeckVisualState; 2]) {
    ui.vertical_centered(|ui| {
        // Tempo controls: [▲] [display] [▼]  (24+5+80+5+24 = 138px)
        ui.horizontal(|ui| {
            ui.set_height(40.0);
            ui.spacing_mut().item_spacing = egui::vec2(5.0, 0.0);
            let content_w = 24.0 + 5.0 + 80.0 + 5.0 + 24.0;
            let pad = ((ui.available_width() - content_w) / 2.0).max(0.0);
            ui.add_space(pad);
            draw_tempo_arrow(ui, "▲", 1.0);
            draw_tempo_display(ui, state.master_tempo);
            draw_tempo_arrow(ui, "▼", -1.0);
        });

        ui.add_space(10.0);

        // Level meters area
        draw_meters_section(ui, state, visuals);
    });
}

fn draw_tempo_arrow(ui: &mut egui::Ui, symbol: &str, delta: f32) {
    // Use 24x40 allocation to match tempo display height, draw 24x24 button centered vertically
    let alloc_size = egui::vec2(24.0, 40.0);
    let btn_size = egui::vec2(24.0, 24.0);
    let (outer_rect, resp) = ui.allocate_exact_size(alloc_size, egui::Sense::click());
    let rect = egui::Rect::from_center_size(outer_rect.center(), btn_size);
    let painter = ui.painter();

    if resp.clicked() {
        let current = CONSOLE_VISUAL.lock().unwrap().master_tempo;
        push_action(UiAction::SetMasterTempo(current + delta));
    }
    let hover = resp.hovered();
    let bg_top = if hover { egui::Color32::from_rgb(85, 85, 85) } else { egui::Color32::from_rgb(68, 68, 68) };
    let bg_bot = if hover { egui::Color32::from_rgb(55, 55, 55) } else { egui::Color32::from_rgb(42, 42, 42) };
    let draw_rect = if resp.is_pointer_button_down_on() { rect.translate(egui::vec2(0.0, 1.0)) } else { rect };
    paint_gradient_135(painter, draw_rect, bg_top, bg_bot, 3.0);
    painter.rect_stroke(draw_rect, 3.0, egui::Stroke::new(1.0, BORDER_MED), egui::StrokeKind::Outside);
    painter.text(
        draw_rect.center(),
        egui::Align2::CENTER_CENTER,
        symbol,
        egui::FontId::proportional(10.0),
        CYAN,
    );
}

fn draw_tempo_display(ui: &mut egui::Ui, tempo: f32) {
    let size = egui::vec2(80.0, 40.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();

    painter.rect_filled(rect, 2.0, egui::Color32::BLACK);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(2.0, egui::Color32::from_rgb(26, 26, 26)),
        egui::StrokeKind::Outside,
    );

    let text = format!("{}", tempo.round().clamp(0.0, 999.0) as i32);
    let font = egui::FontId::new(24.0, egui::FontFamily::Name("dseg7".into()));
    // Glow effect (text-shadow: 0 0 8px #ff4500)
    let glow_color = egui::Color32::from_rgba_unmultiplied(255, 69, 0, 60);
    for &dx in &[-1.0_f32, 0.0, 1.0] {
        for &dy in &[-1.0_f32, 0.0, 1.0] {
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            painter.text(
                rect.center() + egui::vec2(dx, dy),
                egui::Align2::CENTER_CENTER,
                &text,
                font.clone(),
                glow_color,
            );
        }
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        &text,
        font,
        ORANGE,
    );
}

fn draw_meters_section(ui: &mut egui::Ui, state: &ConsoleVisualState, visuals: &[DeckVisualState; 2]) {
    // Column widths: eq=32, vol=18, meter/cue=32
    let col_eq = 32.0;
    let col_vol = 18.0;
    let col_meter = 32.0; // CUE button width; level meter (10px) centered inside
    let sp = 5.0;
    let col_h = 100.0; // uniform height for all columns
    let content_w = col_eq + sp + col_vol + sp + col_meter + sp + col_meter + sp + col_vol + sp + col_eq;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(sp, 0.0);
        let pad = ((ui.available_width() - content_w) / 2.0).max(0.0);
        ui.add_space(pad);

        // EQ kills A
        draw_eq_kills_column(ui, 1, state.deck_a.eq_high, state.deck_a.eq_mid, state.deck_a.eq_low);

        // Volume A + gain %
        ui.allocate_ui_with_layout(
            egui::vec2(col_vol, col_h),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                draw_volume_slider(ui, 1, state.deck_a.gain, 18.0, 70.0);
                let pct = format!("{}%", (state.deck_a.gain * 100.0).round() as i32);
                ui.add(egui::Label::new(egui::RichText::new(pct).size(8.0).color(TEXT_DIM)));
            },
        );

        // Meter A + Cue A
        ui.allocate_ui_with_layout(
            egui::vec2(col_meter, col_h),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                draw_level_meter(ui, state.deck_a.peak, visuals[0].peak_hold, 10.0, 60.0);
                draw_cue_button(ui, 1, state.deck_a.cue_enabled);
            },
        );

        // Meter B + Cue B
        ui.allocate_ui_with_layout(
            egui::vec2(col_meter, col_h),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                draw_level_meter(ui, state.deck_b.peak, visuals[1].peak_hold, 10.0, 60.0);
                draw_cue_button(ui, 2, state.deck_b.cue_enabled);
            },
        );

        // Volume B + gain %
        ui.allocate_ui_with_layout(
            egui::vec2(col_vol, col_h),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                draw_volume_slider(ui, 2, state.deck_b.gain, 18.0, 70.0);
                let pct = format!("{}%", (state.deck_b.gain * 100.0).round() as i32);
                ui.add(egui::Label::new(egui::RichText::new(pct).size(8.0).color(TEXT_DIM)));
            },
        );

        // EQ kills B
        draw_eq_kills_column(ui, 2, state.deck_b.eq_high, state.deck_b.eq_mid, state.deck_b.eq_low);
    });
}

// ── Crossfader ─────────────────────────────────────────────────────────────

fn draw_crossfader(ui: &mut egui::Ui, position: f32) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::click_and_drag());
    let painter = ui.painter();

    // Handle drag/click on crossfader
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let new_pos = ((pos.x - rect.min.x) / width).clamp(0.0, 1.0);
            push_action(UiAction::SetCrossfader(new_pos));
        }
    }

    let track_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(width, 6.0));

    // Gradient track: orange → gray → cyan
    let mid_x = track_rect.center().x;
    paint_gradient_h(
        painter,
        egui::Rect::from_min_max(track_rect.left_top(), egui::pos2(mid_x, track_rect.bottom())),
        ORANGE_SEC,
        egui::Color32::from_rgb(51, 51, 51),
    );
    paint_gradient_h(
        painter,
        egui::Rect::from_min_max(
            egui::pos2(mid_x, track_rect.top()),
            track_rect.right_bottom(),
        ),
        egui::Color32::from_rgb(51, 51, 51),
        CYAN,
    );
    // Rounded track ends
    painter.rect_stroke(track_rect, 3.0, egui::Stroke::new(0.5, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80)), egui::StrokeKind::Outside);

    // Slider handle with gradient
    let sx = track_rect.min.x + position.clamp(0.0, 1.0) * width;
    let handle = egui::Rect::from_center_size(egui::pos2(sx, rect.center().y), egui::vec2(10.0, 24.0));
    paint_gradient_135(painter, handle, egui::Color32::from_rgb(85, 85, 85), egui::Color32::from_rgb(42, 42, 42), 2.0);
    painter.rect_stroke(
        handle,
        2.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(119, 119, 119)),
        egui::StrokeKind::Outside,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Main UI builder
// ═══════════════════════════════════════════════════════════════════════════

// ── Artwork texture cache (render thread only) ────────────────────────────
thread_local! {
    static ARTWORK_TEXTURES: std::cell::RefCell<[(Option<egui::TextureHandle>, u64); 2]> =
        const { std::cell::RefCell::new([(None, 0), (None, 0)]) };
}

fn get_artwork_texture(ctx: &egui::Context, deck_index: usize) -> Option<egui::TextureHandle> {
    let current_version = ARTWORK_VERSIONS[deck_index].load(Ordering::Relaxed);
    ARTWORK_TEXTURES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache[deck_index].1 != current_version {
            // Version changed, need to update texture
            let artwork = DECK_ARTWORK.lock().unwrap();
            if let Some((w, h, ref rgba)) = artwork[deck_index] {
                let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba);
                let tex = ctx.load_texture(
                    format!("deck-art-{}", deck_index),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                cache[deck_index] = (Some(tex), current_version);
            } else {
                cache[deck_index] = (None, current_version);
            }
        }
        cache[deck_index].0.clone()
    })
}

// ── Titlebar ─────────────────────────────────────────────────────────────────

/// Main titlebar panel — 38 px height, matches the Electron CSS reference.
fn draw_titlebar(ctx: &egui::Context, tb: &crate::ui_state::TitlebarState) {
    const H: f32 = 38.0;

    egui::TopBottomPanel::top("titlebar")
        .exact_height(H)
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let full_rect = ui.max_rect();
            let painter = ui.painter();

            // ── Background gradient #1a1a1a → #0f0f0f ────────────────────────
            {
                let c_top = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
                let c_bot = egui::Color32::from_rgb(0x0f, 0x0f, 0x0f);
                let tl = full_rect.left_top();
                let tr = full_rect.right_top();
                let bl = full_rect.left_bottom();
                let br = full_rect.right_bottom();
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(tl, c_top);
                mesh.colored_vertex(tr, c_top);
                mesh.colored_vertex(br, c_bot);
                mesh.colored_vertex(bl, c_bot);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(egui::Shape::Mesh(std::sync::Arc::new(mesh)));
            }

            // ── Bottom border 1 px #00d4ff + soft glow ────────────────────────
            let border_y = full_rect.max.y - 1.0;
            painter.hline(
                full_rect.x_range(),
                border_y,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 212, 255)),
            );
            for i in 1u8..=3 {
                let alpha = 70u8.saturating_sub(i * 22);
                painter.hline(
                    full_rect.x_range(),
                    border_y + i as f32,
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 212, 255, alpha)),
                );
            }

            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0); // right padding

                // ── Time ──────────────────────────────────────────────────────
                ui.label(
                    egui::RichText::new(&tb.time_text)
                        .size(11.0)
                        .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa))
                        .family(egui::FontFamily::Monospace),
                );
                tb_thin_sep(ui);

                // ── MEM ───────────────────────────────────────────────────────
                let mem_text = if tb.mem_mb >= 1024 {
                    format!("{:.1}G", tb.mem_mb as f32 / 1024.0)
                } else {
                    format!("{}M", tb.mem_mb)
                };
                ui.label(
                    egui::RichText::new(mem_text)
                        .size(10.0)
                        .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa))
                        .family(egui::FontFamily::Monospace),
                );
                ui.label(
                    egui::RichText::new("MEM")
                        .size(10.0)
                        .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa)),
                );
                tb_thin_sep(ui);

                // ── CPU value + bar + label ───────────────────────────────────
                ui.label(
                    egui::RichText::new(format!("{:.1}%", tb.cpu_percent))
                        .size(10.0)
                        .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa))
                        .family(egui::FontFamily::Monospace),
                );
                {
                    let (bar_r, _) =
                        ui.allocate_exact_size(egui::vec2(50.0, 8.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(
                        bar_r,
                        4.0,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 38),
                    );
                    let fill_w = bar_r.width() * (tb.cpu_percent / 100.0).clamp(0.0, 1.0);
                    if fill_w > 1.0 {
                        let mid = bar_r.min.x + fill_w * 0.5;
                        let lh = egui::Rect::from_x_y_ranges(
                            bar_r.min.x..=mid,
                            bar_r.y_range(),
                        );
                        let rh = egui::Rect::from_x_y_ranges(
                            mid..=(bar_r.min.x + fill_w),
                            bar_r.y_range(),
                        );
                        p.rect_filled(lh, 0.0, egui::Color32::from_rgb(0x4a, 0x9e, 0xff));
                        p.rect_filled(rh, 0.0, egui::Color32::from_rgb(0xff, 0x6b, 0x6b));
                    }
                }
                ui.label(
                    egui::RichText::new("CPU")
                        .size(10.0)
                        .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa)),
                );

                // ── Section separator ─────────────────────────────────────────
                tb_section_sep(ui);

                // ── MIC level bar (56×6 px, always visible) ──────────────────
                {
                    let (bar_r, _) =
                        ui.allocate_exact_size(egui::vec2(56.0, 6.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(
                        bar_r,
                        3.0,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 25),
                    );
                    let fill_w = bar_r.width() * tb.mic_peak.clamp(0.0, 1.0);
                    if fill_w > 1.0 {
                        let mid = bar_r.min.x + fill_w * 0.5;
                        let lh = egui::Rect::from_x_y_ranges(
                            bar_r.min.x..=mid,
                            bar_r.y_range(),
                        );
                        let rh = egui::Rect::from_x_y_ranges(
                            mid..=(bar_r.min.x + fill_w),
                            bar_r.y_range(),
                        );
                        p.rect_filled(lh, 0.0, egui::Color32::from_rgb(0x21, 0xd4, 0xfd));
                        p.rect_filled(rh, 0.0, egui::Color32::from_rgb(0xff, 0x6b, 0x6b));
                    }
                }

                // ── MIC pill button ───────────────────────────────────────────
                let mic_active = tb.mic_enabled;
                let (mic_border, mic_dot, mic_text, mic_bg_top, mic_bg_bot) = if mic_active {
                    (
                        egui::Color32::from_rgb(0x4d, 0xff, 0x4d),
                        egui::Color32::from_rgb(0x3b, 0xff, 0x3b),
                        egui::Color32::from_rgb(0xd2, 0xff, 0xd2),
                        egui::Color32::from_rgb(0x20, 0x3a, 0x26),
                        egui::Color32::from_rgb(0x12, 0x20, 0x1a),
                    )
                } else {
                    (
                        egui::Color32::from_rgb(0x2a, 0x60, 0x30),
                        egui::Color32::from_rgb(0x33, 0x55, 0x33),
                        egui::Color32::from_rgb(0x70, 0xb0, 0x80),
                        egui::Color32::from_rgb(0x1a, 0x36, 0x20),
                        egui::Color32::from_rgb(0x0f, 0x1f, 0x12),
                    )
                };
                let mic_resp = tb_pill_button(
                    ui,
                    "MIC",
                    mic_active,
                    mic_border,
                    mic_dot,
                    mic_text,
                    mic_bg_top,
                    mic_bg_bot,
                    tb.mic_available,
                );
                if mic_resp.clicked() && tb.mic_available {
                    push_action(UiAction::SetMicEnabled(!tb.mic_enabled));
                }

                // ── Section separator ─────────────────────────────────────────
                tb_section_sep(ui);

                // ── REC pill button ───────────────────────────────────────────
                let rec_active = tb.is_recording;
                let rec_label = if rec_active {
                    let h = tb.rec_elapsed_secs / 3600;
                    let m = (tb.rec_elapsed_secs % 3600) / 60;
                    let s = tb.rec_elapsed_secs % 60;
                    if h > 0 {
                        format!("REC {:02}:{:02}:{:02}", h, m, s)
                    } else {
                        format!("REC {:02}:{:02}", m, s)
                    }
                } else {
                    "REC".to_owned()
                };
                let (rec_border, rec_dot, rec_text, rec_bg_top, rec_bg_bot) = if rec_active {
                    (
                        egui::Color32::from_rgb(0xff, 0x4d, 0x4d),
                        egui::Color32::from_rgb(0xff, 0x3b, 0x3b),
                        egui::Color32::from_rgb(0xff, 0xd2, 0xd2),
                        egui::Color32::from_rgb(0x4a, 0x1e, 0x1e),
                        egui::Color32::from_rgb(0x2d, 0x0e, 0x0e),
                    )
                } else {
                    (
                        egui::Color32::from_rgb(0xb0, 0x30, 0x30),
                        egui::Color32::from_rgb(0x55, 0x33, 0x33),
                        egui::Color32::from_rgb(0xff, 0x85, 0x85),
                        egui::Color32::from_rgb(0x36, 0x18, 0x18),
                        egui::Color32::from_rgb(0x22, 0x0b, 0x0b),
                    )
                };
                let rec_resp = tb_pill_button(
                    ui,
                    &rec_label,
                    rec_active,
                    rec_border,
                    rec_dot,
                    rec_text,
                    rec_bg_top,
                    rec_bg_bot,
                    true,
                );
                if rec_resp.clicked() {
                    if rec_active {
                        push_action(UiAction::StopRecording);
                    } else {
                        push_action(UiAction::StartRecording);
                    }
                }

                // ── Left: 80 px gap (traffic lights) + "Sujay" title ──────────
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(80.0);
                    ui.label(
                        egui::RichText::new("Sujay")
                            .size(13.0)
                            .strong()
                            .color(egui::Color32::from_rgb(0, 212, 255)),
                    );
                });
            });
        });
}

fn set_channel_with_uniqueness(
    prefs: &mut PreferencesState,
    target_main: bool,
    side: usize,
    value: Option<i32>,
) {
    let mut main = prefs.main_channels;
    let mut cue = prefs.cue_channels;

    if target_main {
        main[side] = value;
    } else {
        cue[side] = value;
    }

    if let Some(v) = value {
        for idx in 0..2 {
            if !(target_main && idx == side) && main[idx] == Some(v) {
                main[idx] = None;
            }
            if !(!target_main && idx == side) && cue[idx] == Some(v) {
                cue[idx] = None;
            }
        }
    }

    prefs.main_channels = main;
    prefs.cue_channels = cue;
}

fn draw_channel_pair(
    ui: &mut egui::Ui,
    title: &str,
    pair: [Option<i32>; 2],
    target_main: bool,
    max_channels: i32,
    draft: &mut PreferencesState,
) {
    ui.group(|ui| {
        ui.label(title);
        ui.horizontal(|ui| {
            for side in 0..2 {
                let current = pair[side].unwrap_or(-1);
                egui::ComboBox::from_id_salt((title, side))
                    .selected_text(if current < 0 { "-".to_owned() } else { (current + 1).to_string() })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current < 0, "-").clicked() {
                            set_channel_with_uniqueness(draft, target_main, side, None);
                        }
                        for channel in 0..max_channels {
                            let selected = current == channel;
                            if ui.selectable_label(selected, (channel + 1).to_string()).clicked() {
                                set_channel_with_uniqueness(draft, target_main, side, Some(channel));
                            }
                        }
                    });
            }
        });
    });
}

fn draw_preferences_modal(ctx: &egui::Context) {
    if !PREFS_OPEN.load(Ordering::Relaxed) {
        return;
    }

    let mut open = true;
    egui::Window::new("Preferences")
        .id(egui::Id::new("preferences_modal"))
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(640.0, 440.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            let mut draft = PREFS_DRAFT.lock().unwrap().clone();

            ui.horizontal(|ui| {
                for (idx, name) in ["Audio", "Recording", "OSC"].iter().enumerate() {
                    let is_active = PREFS_TAB.load(Ordering::Relaxed) == idx as u8;
                    if ui.selectable_label(is_active, *name).clicked() {
                        PREFS_TAB.store(idx as u8, Ordering::Relaxed);
                    }
                }
            });

            ui.separator();
            ui.add_space(4.0);

            match PREFS_TAB.load(Ordering::Relaxed) {
                0 => {
                    ui.label("Audio Device");
                    egui::ComboBox::from_id_salt("pref_device")
                        .selected_text(
                            draft.audio_device_id.clone().unwrap_or_else(|| "System Default".to_owned())
                        )
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(draft.audio_device_id.is_none(), "System Default").clicked() {
                                draft.audio_device_id = None;
                            }
                            for dev in &draft.audio_devices {
                                let selected = draft.audio_device_id.as_ref() == Some(&dev.name);
                                let label = format!("{} ({} ch)", dev.name, dev.max_output_channels);
                                if ui.selectable_label(selected, label).clicked() {
                                    draft.audio_device_id = Some(dev.name.clone());
                                }
                            }
                        });

                    let max_channels = draft
                        .audio_device_id
                        .as_ref()
                        .and_then(|name| draft.audio_devices.iter().find(|d| &d.name == name))
                        .map(|d| d.max_output_channels as i32)
                        .unwrap_or(2);

                    ui.add_space(8.0);
                    ui.label("Output Routing");

                    ui.horizontal(|ui| {
                        draw_channel_pair(ui, "Main Output", draft.main_channels, true, max_channels, &mut draft);
                        draw_channel_pair(ui, "Cue Output", draft.cue_channels, false, max_channels, &mut draft);
                    });
                }
                1 => {
                    ui.label("Recording Directory");
                    ui.text_edit_singleline(&mut draft.recording_directory);
                    ui.checkbox(
                        &mut draft.recording_auto_create_directory,
                        "Auto-create recording directory",
                    );
                    ui.add_space(8.0);
                    ui.label("Recording Naming");
                    egui::ComboBox::from_id_salt("pref_rec_naming")
                        .selected_text(draft.recording_naming_strategy.clone())
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    draft.recording_naming_strategy == "timestamp",
                                    "timestamp",
                                )
                                .clicked()
                            {
                                draft.recording_naming_strategy = "timestamp".to_owned();
                            }
                            if ui
                                .selectable_label(
                                    draft.recording_naming_strategy == "sequential",
                                    "sequential",
                                )
                                .clicked()
                            {
                                draft.recording_naming_strategy = "sequential".to_owned();
                            }
                        });
                    ui.add_space(8.0);
                    ui.label("Recording Format");
                    egui::ComboBox::from_id_salt("pref_rec_format")
                        .selected_text(draft.recording_format.clone())
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(draft.recording_format == "wav", "wav").clicked() {
                                draft.recording_format = "wav".to_owned();
                            }
                            if ui.selectable_label(draft.recording_format == "ogg", "ogg").clicked() {
                                draft.recording_format = "ogg".to_owned();
                            }
                        });
                }
                2 => {
                    ui.checkbox(&mut draft.osc_enabled, "Enable OSC");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Host");
                        ui.text_edit_singleline(&mut draft.osc_host);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        let mut port = draft.osc_port.to_string();
                        if ui.text_edit_singleline(&mut port).changed() {
                            if let Ok(parsed) = port.parse::<u16>() {
                                if parsed > 0 {
                                    draft.osc_port = parsed;
                                }
                            }
                        }
                    });
                }
                _ => {}
            }

            ui.add_space(12.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    PREFS_OPEN.store(false, Ordering::Relaxed);
                }
                if ui.button("Save").clicked() {
                    *PREFS_VISUAL.lock().unwrap() = draft.clone();
                    push_action(UiAction::SavePreferences(draft.clone()));
                    PREFS_OPEN.store(false, Ordering::Relaxed);
                }
            });

            *PREFS_DRAFT.lock().unwrap() = draft;
        });

    if !open {
        PREFS_OPEN.store(false, Ordering::Relaxed);
    }
}

/// Custom pill-shaped button used for REC and MIC indicators.
/// Returns the egui Response so the caller can test `.clicked()`.
fn tb_pill_button(
    ui: &mut egui::Ui,
    label: &str,
    active: bool,
    border_color: egui::Color32,
    dot_color: egui::Color32,
    text_color: egui::Color32,
    bg_top: egui::Color32,
    bg_bot: egui::Color32,
    enabled: bool,
) -> egui::Response {
    let font_id = egui::FontId::new(11.0, egui::FontFamily::Proportional);
    let galley =
        ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font_id, text_color));

    const DOT_D: f32 = 8.0;
    const DOT_GAP: f32 = 6.0;
    const PAD_X: f32 = 10.0;
    const PAD_Y: f32 = 4.0;

    let content_w = DOT_D + DOT_GAP + galley.rect.width();
    let btn_w = (PAD_X * 2.0 + content_w).max(60.0);
    let btn_h = (PAD_Y * 2.0 + galley.rect.height()).max(22.0);
    let rounding: f32 = btn_h / 2.0;

    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(btn_w, btn_h), sense);
    let p = ui.painter();

    // Background gradient (top lighter, bottom darker) via Mesh
    {
        let tl = rect.left_top();
        let tr = rect.right_top();
        let bl = rect.left_bottom();
        let br = rect.right_bottom();
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(tl, bg_top);
        mesh.colored_vertex(tr, bg_top);
        mesh.colored_vertex(br, bg_bot);
        mesh.colored_vertex(bl, bg_bot);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(egui::Shape::Mesh(std::sync::Arc::new(mesh)));
    }
    // Solid fill with rounding (covers gradient corners)
    p.rect_filled(rect, rounding, bg_bot);

    // Active glow ring
    if active {
        let glow_alpha = 70u8;
        let glow = egui::Color32::from_rgba_unmultiplied(
            border_color.r(),
            border_color.g(),
            border_color.b(),
            glow_alpha,
        );
        p.rect_stroke(rect, rounding, egui::Stroke::new(3.0, glow), egui::StrokeKind::Outside);
    }

    // 1 px border
    p.rect_stroke(rect, rounding, egui::Stroke::new(1.0, border_color), egui::StrokeKind::Outside);

    // Dot indicator (8 px circle)
    let dot_cx = rect.min.x + PAD_X + DOT_D / 2.0;
    let dot_cy = rect.center().y;
    p.circle_filled(egui::pos2(dot_cx, dot_cy), DOT_D / 2.0, dot_color);

    // Label text
    let text_x = dot_cx + DOT_D / 2.0 + DOT_GAP;
    let text_y = rect.center().y - galley.rect.height() / 2.0;
    p.galley(egui::pos2(text_x, text_y), galley, text_color);

    resp
}

/// 1×16 px hairline separator between CPU / MEM / time items.
#[inline]
fn tb_thin_sep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 16.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 25));
}

/// 1×22 px separator between the pill-button section and the metrics section.
#[inline]
fn tb_section_sep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 22.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 25));
}

fn build_console_ui(ctx: &egui::Context) {
    let console = CONSOLE_VISUAL.lock().unwrap().clone();
    let waveforms = WAVEFORMS.lock().unwrap().clone();
    let visuals = DECK_VISUALS.lock().unwrap().clone();

    draw_titlebar(ctx, &console.titlebar);

    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(BG_DARK)
                .inner_margin(10.0)
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(51, 51, 51)))
        )
        .show(ctx, |ui| {
            // Paint 135° gradient background over the panel
            let bg_rect = ui.max_rect();
            let painter = ui.painter();
            paint_gradient_135(
                painter,
                bg_rect,
                egui::Color32::from_rgb(26, 26, 26),
                egui::Color32::from_rgb(15, 15, 15),
                0.0,
            );

            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

            // ── Top: Zoom waveforms ──
            draw_zoom_waveform(ui, &waveforms[0], &visuals[0], &console.deck_a, console.master_tempo);
            ui.add_space(5.0);
            draw_zoom_waveform(ui, &waveforms[1], &visuals[1], &console.deck_b, console.master_tempo);
            ui.add_space(12.0);

            // ── Middle: Decks + Tempo ──
            let available = ui.available_size();
            let crossfader_reserve = 48.0;
            let max_deck_h = (available.y - crossfader_reserve).max(156.0);
            let deck_area_h = max_deck_h.min(176.0);
            let tempo_w = 200.0;
            let gap = 8.0;
            let side_margin = 4.0;
            let deck_w = ((available.x - tempo_w - gap * 2.0 - side_margin) * 0.5).max(100.0);

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(deck_w, deck_area_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| draw_deck(ui, true, &console.deck_a, &waveforms[0], &visuals[0]),
                );
                ui.add_space(gap);
                ui.allocate_ui_with_layout(
                    egui::vec2(tempo_w, deck_area_h),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| draw_tempo_section(ui, &console, &visuals),
                );
                ui.add_space(gap);
                ui.allocate_ui_with_layout(
                    egui::vec2(deck_w, deck_area_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| draw_deck(ui, false, &console.deck_b, &waveforms[1], &visuals[1]),
                );
            });

            // ── Bottom: Crossfader ──
            ui.add_space(8.0);
            draw_crossfader(ui, console.crossfader);
        });

    draw_preferences_modal(ctx);
}

/// Push a mouse event from the host windowing system (winit) directly into the renderer.
///
/// `kind`: 0 = cursor moved, 1 = left-button pressed, 2 = left-button released.
/// `x`, `y`: logical points, top-left origin, relative to the view's top-left corner.
pub fn push_mouse_event(kind: u8, x: f32, y: f32) {
    let event = match kind {
        1 => MouseEvent::Pressed(x, y),
        2 => MouseEvent::Released(x, y),
        _ => MouseEvent::Moved(x, y),
    };
    MOUSE_EVENTS.lock().unwrap().push(event);
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
}

pub fn set_waveform(deck_index: usize, samples: Vec<f32>) {
    if deck_index > 1 {
        return;
    }
    let mut guard = WAVEFORMS.lock().unwrap();
    guard[deck_index] = samples;
    WAVEFORM_VERSIONS[deck_index].fetch_add(1, Ordering::Relaxed);
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
}

pub fn set_deck_progress(deck_index: usize, position_frames: f32, total_frames: f32, audio_sample_rate: f32) {
    if deck_index > 1 {
        return;
    }
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
    let mut guard = DECK_VISUALS.lock().unwrap();
    guard[deck_index].progress = position_frames.clamp(0.0, total_frames);
    guard[deck_index].total_frames = total_frames;
    guard[deck_index].audio_sample_rate = audio_sample_rate;
    // Peak hold: track max peak, decay slowly
    let console = CONSOLE_VISUAL.lock().unwrap();
    let current_peak = if deck_index == 0 { console.deck_a.peak } else { console.deck_b.peak };
    drop(console);
    if current_peak > guard[deck_index].peak_hold {
        guard[deck_index].peak_hold = current_peak;
    } else {
        // Decay: ~1.5 seconds from full to zero at 60fps
        guard[deck_index].peak_hold = (guard[deck_index].peak_hold - 0.012).max(0.0);
    }
}

pub fn set_deck_markers(
    deck_index: usize,
    beats: Vec<f32>,
    intro: Option<f32>,
    outro: Option<f32>,
) {
    if deck_index > 1 {
        return;
    }
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
    let mut guard = DECK_VISUALS.lock().unwrap();
    // All values are already audio frame indices — store directly
    guard[deck_index].beats = beats
        .into_iter()
        .filter(|v| v.is_finite())
        .collect();
    guard[deck_index].intro = intro.filter(|v| v.is_finite());
    guard[deck_index].outro = outro.filter(|v| v.is_finite());
}

pub fn set_console_state(state: ConsoleVisualState) {
    // All loop positions are audio frame indices — store directly
    let mut guard = CONSOLE_VISUAL.lock().unwrap();
    if *guard == state {
        return;
    }
    *guard = state;
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
}

pub fn set_preferences_state(state: PreferencesState) {
    let mut guard = PREFS_VISUAL.lock().unwrap();
    if *guard != state {
        *guard = state.clone();
        if !PREFS_OPEN.load(Ordering::Relaxed) {
            *PREFS_DRAFT.lock().unwrap() = state;
        }
        NEEDS_REPAINT.store(true, Ordering::Relaxed);
    }
}

#[allow(dead_code)]
pub fn set_deck_artwork(deck_index: usize, width: u32, height: u32, rgba: Vec<u8>) {
    if deck_index > 1 {
        return;
    }
    let mut guard = DECK_ARTWORK.lock().unwrap();
    guard[deck_index] = Some((width, height, rgba));
    ARTWORK_VERSIONS[deck_index].fetch_add(1, Ordering::Relaxed);
    NEEDS_REPAINT.store(true, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn clear_deck_artwork(deck_index: usize) {
    if deck_index > 1 {
        return;
    }
    let mut guard = DECK_ARTWORK.lock().unwrap();
    guard[deck_index] = None;
    ARTWORK_VERSIONS[deck_index].fetch_add(1, Ordering::Relaxed);
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared render loop (wgpu + egui_wgpu) — platform-independent
// ═══════════════════════════════════════════════════════════════════════════

/// Drive the egui console on a ready `wgpu` surface until `running` clears.
///
/// Callers create the surface/adapter/device on whichever platform handle they
/// own (NSView on macOS, HWND on Windows), configure it once, then hand the
/// pieces here. `pending_size` carries `(width_px, height_px, pixels_per_point)`
/// and is polled every frame so host resizes / DPI changes take effect live.
pub(crate) fn run_egui_render_loop(
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    mut config: wgpu::SurfaceConfiguration,
    running: Arc<AtomicBool>,
    pending_size: Arc<Mutex<(u32, u32, f32)>>,
) {
    let egui_ctx = egui::Context::default();
    setup_fonts(&egui_ctx);
    setup_style(&egui_ctx);

    let mut egui_renderer = egui_wgpu::Renderer::new(&device, format, None, 1, false);
    let start_time = Instant::now();
    let frame_duration = Duration::from_secs_f64(1.0 / 60.0);

    while running.load(Ordering::Relaxed) {
        let frame_start = Instant::now();
        let (latest_w, latest_h, latest_s) = *pending_size.lock().unwrap();

        // Check if either deck is playing – if so, always repaint for smooth scrubbing
        let any_playing = {
            let c = CONSOLE_VISUAL.lock().unwrap();
            c.deck_a.playing || c.deck_b.playing
        };
        let has_mouse_events = !MOUSE_EVENTS.lock().unwrap().is_empty();
        let needs_repaint = NEEDS_REPAINT.load(Ordering::Relaxed);

        if !any_playing && !needs_repaint && !has_mouse_events {
            // Nothing to do – sleep and try again
            thread::sleep(frame_duration);
            continue;
        }
        // Clear dirty flag before rendering so concurrent updates set it again
        NEEDS_REPAINT.store(false, Ordering::Relaxed);

        if latest_w != config.width || latest_h != config.height {
            config.width = latest_w.max(1);
            config.height = latest_h.max(1);
            surface.configure(&device, &config);
        }

        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&device, &config);
                continue;
            }
            Err(wgpu::SurfaceError::Timeout) => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                eprintln!("[native-ui] wgpu surface out of memory");
                break;
            }
            Err(e) => {
                eprintln!("[native-ui] wgpu surface error: {e}");
                break;
            }
        };

        let texture_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [config.width, config.height],
            pixels_per_point: latest_s,
        };

        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(
                    config.width as f32 / latest_s,
                    config.height as f32 / latest_s,
                ),
            )),
            time: Some(start_time.elapsed().as_secs_f64()),
            ..Default::default()
        };
        let mut viewport_info = egui::ViewportInfo::default();
        viewport_info.native_pixels_per_point = Some(latest_s);
        raw_input.viewports.insert(raw_input.viewport_id, viewport_info);

        // Inject mouse events from the host window
        {
            let mut events = MOUSE_EVENTS.lock().unwrap();
            for me in events.drain(..) {
                match me {
                    MouseEvent::Moved(x, y) => {
                        raw_input.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
                    }
                    MouseEvent::Pressed(x, y) => {
                        raw_input.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
                        raw_input.events.push(egui::Event::PointerButton {
                            pos: egui::pos2(x, y),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                    MouseEvent::Released(x, y) => {
                        raw_input.events.push(egui::Event::PointerButton {
                            pos: egui::pos2(x, y),
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
            }
        }

        let full_output = egui_ctx.run(raw_input, |ctx| {
            build_console_ui(ctx);
        });

        let clipped = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        for (id, delta) in &full_output.textures_delta.set {
            egui_renderer.update_texture(&device, &queue, *id, delta);
        }

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sujay-egui-encoder"),
            });

        egui_renderer.update_buffers(&device, &queue, &mut encoder, &clipped, &screen_desc);

        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sujay-egui-render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.04,
                            g: 0.04,
                            b: 0.04,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut pass = pass.forget_lifetime();
            egui_renderer.render(&mut pass, &clipped, &screen_desc);
        }

        queue.submit(Some(encoder.finish()));
        frame.present();

        for id in &full_output.textures_delta.free {
            egui_renderer.free_texture(id);
        }

        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            thread::sleep(frame_duration - elapsed);
        }
    }
}
