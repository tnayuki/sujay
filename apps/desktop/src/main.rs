#![allow(unexpected_cfgs)]

//! Sujay — Phase 5 Rust-native host binary.
//!
//! Creates a winit window, attaches the wgpu/egui UI renderer, and drives the
//! Rust AudioEngineCore — no Electron, no Node.js, no NAPI.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
// Used by the macOS menu's PREFS_REQUESTED flag; the Windows menu module has its own.
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant, SystemTime};
use tracing_subscriber::EnvFilter;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[cfg(any(target_os = "macos", target_os = "windows"))]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(target_os = "windows")]
mod win_settings;

#[cfg(target_os = "macos")]
mod mac_settings;

#[cfg(target_os = "macos")]
use cocoa::base::{id, nil};
#[cfg(target_os = "macos")]
use cocoa::foundation::NSString;
#[cfg(target_os = "macos")]
use objc::declare::ClassDecl;
#[cfg(target_os = "macos")]
use objc::runtime::{Class, Object, Sel};
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};

use sujay_audio::engine_core::{
    list_output_devices, AudioEngineCore, DeviceConfigCore, EngineStateUpdate,
};
#[cfg(not(target_os = "macos"))]
use sujay_decks::push_mouse_event_raw;
use sujay_decks::{
    attach_raw, detach_raw, poll_actions_raw, set_console_state_raw, set_deck_progress_raw,
    set_frame_raw, set_library_state_raw, set_preferences_state_raw,
};

#[cfg(target_os = "macos")]
static PREFS_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static LIBRARY_MENU_SELECTION: AtomicU8 = AtomicU8::new(0);

// ── App state ────────────────────────────────────────────────────────────────

/// Result of a background decode, sent back to the main thread for loading.
struct DecodeReady {
    deck: u8,
    pcm: Vec<f32>,
    waveform: Vec<f32>,
    waveform_colors: Vec<[u8; 3]>,
    bpm: Option<f32>,
    title: String,
    /// Beat positions in audio frames.
    beats: Vec<f32>,
    /// Intro position in audio frames (if detected).
    intro: Option<f32>,
    /// Outro position in audio frames (if detected).
    outro: Option<f32>,
    cues: Vec<sujay_decks::ui_state::DeckCueVisualState>,
    /// Total mono frames (pcm.len() / 2).
    total_frames: f32,
}

struct DecodeFailure {
    deck: u8,
    path: PathBuf,
    error: String,
}

/// Everything a decode produces, before it is sent to the main thread.
struct DecodedTrack {
    pcm: Vec<f32>,
    sample_rate: u32,
    bpm: Option<f32>,
    beats: Vec<f32>,
    intro: Option<f32>,
    outro: Option<f32>,
    waveform: Vec<f32>,
    waveform_colors: Vec<[u8; 3]>,
    cues: Vec<sujay_decks::ui_state::DeckCueVisualState>,
    title: String,
}

enum DecodeResult {
    Ready(DecodeReady),
    Failed(DecodeFailure),
}

#[derive(Clone, Debug, Default)]
struct RekordboxTrackOverride {
    title: String,
    bpm: Option<f32>,
    beats_ms: Vec<f32>,
    waveform: Vec<f32>,
    waveform_colors: Vec<[u8; 3]>,
    cues: Vec<sujay_library::RekordboxCue>,
}

struct RekordboxLibraryLoadReady {
    master_db_path: PathBuf,
    source_label: String,
    tracks: Vec<sujay_decks::ui_state::LibraryTrackItem>,
    playlists: Vec<sujay_decks::ui_state::LibraryPlaylistItem>,
    overrides: HashMap<PathBuf, RekordboxTrackOverride>,
    track_ids: HashMap<PathBuf, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppPreferences {
    #[serde(default)]
    audio_device_id: Option<String>,
    #[serde(default = "default_main_channels")]
    main_channels: [Option<i32>; 2],
    #[serde(default = "default_cue_channels")]
    cue_channels: [Option<i32>; 2],
    #[serde(default = "default_recording_directory")]
    recording_directory: String,
    #[serde(default = "default_recording_auto_create_directory")]
    recording_auto_create_directory: bool,
    #[serde(default = "default_recording_naming_strategy")]
    recording_naming_strategy: String,
    #[serde(default = "default_recording_format")]
    recording_format: String,
    #[serde(default = "default_osc_enabled")]
    osc_enabled: bool,
    #[serde(default = "default_osc_host")]
    osc_host: String,
    #[serde(default = "default_osc_port")]
    osc_port: u16,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            audio_device_id: None,
            main_channels: default_main_channels(),
            cue_channels: default_cue_channels(),
            recording_directory: default_recording_directory(),
            recording_auto_create_directory: default_recording_auto_create_directory(),
            recording_naming_strategy: default_recording_naming_strategy(),
            recording_format: default_recording_format(),
            osc_enabled: default_osc_enabled(),
            osc_host: default_osc_host(),
            osc_port: default_osc_port(),
        }
    }
}

fn default_main_channels() -> [Option<i32>; 2] {
    [Some(0), Some(1)]
}

fn default_cue_channels() -> [Option<i32>; 2] {
    [None, None]
}

fn default_recording_directory() -> String {
    dirs::audio_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
        .join("Sujay Recordings")
        .to_string_lossy()
        .to_string()
}

fn default_recording_format() -> String {
    "wav".to_owned()
}

fn default_recording_auto_create_directory() -> bool {
    true
}

fn default_recording_naming_strategy() -> String {
    "timestamp".to_owned()
}

fn default_osc_enabled() -> bool {
    false
}

fn default_osc_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_osc_port() -> u16 {
    9000
}

fn normalize_preferences(
    prefs: &mut AppPreferences,
    audio_devices: &[sujay_decks::ui_state::AudioDeviceInfo],
) {
    let selected_max = prefs
        .audio_device_id
        .as_ref()
        .and_then(|id| audio_devices.iter().find(|d| &d.name == id))
        .map(|d| d.max_output_channels as i32)
        .unwrap_or(2)
        .max(2);

    let mut used = HashSet::new();
    for idx in 0..2 {
        if let Some(ch) = prefs.main_channels[idx] {
            if ch < 0 || ch >= selected_max || !used.insert(ch) {
                prefs.main_channels[idx] = None;
            }
        }
    }
    for idx in 0..2 {
        if let Some(ch) = prefs.cue_channels[idx] {
            if ch < 0 || ch >= selected_max || !used.insert(ch) {
                prefs.cue_channels[idx] = None;
            }
        }
    }

    if prefs.main_channels[0].is_none() && prefs.main_channels[1].is_none() {
        prefs.main_channels[0] = Some(0);
        if selected_max > 1 {
            prefs.main_channels[1] = Some(1);
        }
    }

    if prefs.recording_directory.trim().is_empty() {
        prefs.recording_directory = default_recording_directory();
    }
    if prefs.recording_naming_strategy != "timestamp"
        && prefs.recording_naming_strategy != "sequential"
    {
        prefs.recording_naming_strategy = default_recording_naming_strategy();
    }
    if prefs.recording_format != "wav" && prefs.recording_format != "ogg" {
        prefs.recording_format = default_recording_format();
    }
    if prefs.osc_host.trim().is_empty() {
        prefs.osc_host = default_osc_host();
    }
    if prefs.osc_port == 0 {
        prefs.osc_port = default_osc_port();
    }
}

fn apply_preferences_state(
    prefs: &mut AppPreferences,
    state: sujay_decks::ui_state::PreferencesState,
    audio_devices: &[sujay_decks::ui_state::AudioDeviceInfo],
) {
    prefs.audio_device_id = state.audio_device_id;
    prefs.main_channels = state.main_channels;
    prefs.cue_channels = state.cue_channels;
    prefs.recording_directory = state.recording_directory;
    prefs.recording_auto_create_directory = state.recording_auto_create_directory;
    prefs.recording_naming_strategy = state.recording_naming_strategy;
    prefs.recording_format = state.recording_format;
    prefs.osc_enabled = state.osc_enabled;
    prefs.osc_host = state.osc_host;
    prefs.osc_port = state.osc_port;
    normalize_preferences(prefs, audio_devices);
}

fn recording_extension(format: &str) -> &'static str {
    if format == "ogg" {
        "ogg"
    } else {
        "wav"
    }
}

fn prepare_recording_path(prefs: &AppPreferences) -> Result<PathBuf, String> {
    let rec_dir = PathBuf::from(&prefs.recording_directory);
    if !rec_dir.is_absolute() {
        return Err("recording directory must be an absolute path".to_owned());
    }

    if !rec_dir.exists() {
        if !prefs.recording_auto_create_directory {
            return Err(format!(
                "recording directory not found: {}",
                rec_dir.display()
            ));
        }
        fs::create_dir_all(&rec_dir).map_err(|e| e.to_string())?;
    }

    let ext = recording_extension(&prefs.recording_format);
    if prefs.recording_naming_strategy == "sequential" {
        for index in 1..=9999 {
            let path = rec_dir.join(format!("{:04}.{}", index, ext));
            if !path.exists() {
                return Ok(path);
            }
        }
        return Err("unable to allocate recording filename".to_owned());
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let base = format!("sujay_{}", ts);
    for suffix in 0..=999 {
        let name = if suffix == 0 {
            format!("{}.{}", base, ext)
        } else {
            format!("{}_{}.{}", base, suffix, ext)
        };
        let path = rec_dir.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("unable to allocate timestamp recording filename".to_owned())
}

fn settings_file_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
        .join("Sujay")
        .join("settings.json")
}

fn load_preferences(path: &PathBuf) -> AppPreferences {
    fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save_preferences(path: &PathBuf, prefs: &AppPreferences) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid settings path".to_owned())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;

    {
        let mut file = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
        file.write_all(&json).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }

    fs::rename(&tmp_path, path).map_err(|e| e.to_string())
}

fn device_config_from_preferences(prefs: &AppPreferences) -> DeviceConfigCore {
    DeviceConfigCore {
        device_id: prefs.audio_device_id.clone(),
        main_channels: Some(
            prefs
                .main_channels
                .iter()
                .map(|v| v.unwrap_or(-1))
                .collect(),
        ),
        cue_channels: Some(prefs.cue_channels.iter().map(|v| v.unwrap_or(-1)).collect()),
    }
}

fn ui_preferences_state(
    prefs: &AppPreferences,
    audio_devices: &[sujay_decks::ui_state::AudioDeviceInfo],
) -> sujay_decks::ui_state::PreferencesState {
    sujay_decks::ui_state::PreferencesState {
        audio_device_id: prefs.audio_device_id.clone(),
        audio_devices: audio_devices.to_vec(),
        main_channels: prefs.main_channels,
        cue_channels: prefs.cue_channels,
        recording_directory: prefs.recording_directory.clone(),
        recording_auto_create_directory: prefs.recording_auto_create_directory,
        recording_naming_strategy: prefs.recording_naming_strategy.clone(),
        recording_format: prefs.recording_format.clone(),
        osc_enabled: prefs.osc_enabled,
        osc_host: prefs.osc_host.clone(),
        osc_port: prefs.osc_port,
    }
}

fn available_audio_devices() -> Vec<sujay_decks::ui_state::AudioDeviceInfo> {
    list_output_devices()
        .unwrap_or_default()
        .into_iter()
        .map(
            |(name, max_output_channels)| sujay_decks::ui_state::AudioDeviceInfo {
                name,
                max_output_channels,
            },
        )
        .collect()
}

#[cfg(target_os = "macos")]
fn menu_handler_class() -> &'static Class {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let mut decl = ClassDecl::new("SujayMenuHandler", class!(NSObject))
            .expect("create menu handler class");

        extern "C" fn open_preferences(_: &Object, _: Sel, _: id) {
            PREFS_REQUESTED.store(true, Ordering::Relaxed);
        }
        extern "C" fn load_deck_a(_: &Object, _: Sel, _: id) {
            LIBRARY_MENU_SELECTION.store(1, Ordering::Relaxed);
        }
        extern "C" fn load_deck_b(_: &Object, _: Sel, _: id) {
            LIBRARY_MENU_SELECTION.store(2, Ordering::Relaxed);
        }

        unsafe {
            decl.add_method(
                sel!(openPreferences:),
                open_preferences as extern "C" fn(&Object, Sel, id),
            );
            decl.add_method(
                sel!(loadDeckA:),
                load_deck_a as extern "C" fn(&Object, Sel, id),
            );
            decl.add_method(
                sel!(loadDeckB:),
                load_deck_b as extern "C" fn(&Object, Sel, id),
            );
        }
        decl.register();
    });
    Class::get("SujayMenuHandler").expect("SujayMenuHandler class")
}

#[cfg(target_os = "macos")]
unsafe fn show_native_library_context_menu(window: &Window, x: f32, y: f32) -> Option<u8> {
    LIBRARY_MENU_SELECTION.store(0, Ordering::Relaxed);

    let ns_view = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as id,
        _ => return None,
    };

    let menu: id = msg_send![class!(NSMenu), alloc];
    let menu: id = msg_send![menu, initWithTitle: NSString::alloc(nil).init_str("Load")];
    let key_empty = NSString::alloc(nil).init_str("");

    let handler: id = msg_send![menu_handler_class(), new];

    let deck_a_title = NSString::alloc(nil).init_str("Deck A");
    let deck_a: id = msg_send![class!(NSMenuItem), alloc];
    let deck_a: id = msg_send![deck_a, initWithTitle: deck_a_title action: sel!(loadDeckA:) keyEquivalent: key_empty];
    let _: () = msg_send![deck_a, setTarget: handler];
    let _: () = msg_send![menu, addItem: deck_a];

    let deck_b_title = NSString::alloc(nil).init_str("Deck B");
    let deck_b: id = msg_send![class!(NSMenuItem), alloc];
    let deck_b: id = msg_send![deck_b, initWithTitle: deck_b_title action: sel!(loadDeckB:) keyEquivalent: key_empty];
    let _: () = msg_send![deck_b, setTarget: handler];
    let _: () = msg_send![menu, addItem: deck_b];

    let ns_window: id = msg_send![ns_view, window];
    let pos = if ns_window != nil {
        let screen_pos: cocoa::foundation::NSPoint = msg_send![class!(NSEvent), mouseLocation];
        let window_pos: cocoa::foundation::NSPoint =
            msg_send![ns_window, convertPointFromScreen: screen_pos];
        let view_pos: cocoa::foundation::NSPoint =
            msg_send![ns_view, convertPoint: window_pos fromView: nil];
        view_pos
    } else {
        let bounds: cocoa::foundation::NSRect = msg_send![ns_view, bounds];
        cocoa::foundation::NSPoint::new(x as f64, (bounds.size.height - y as f64).max(0.0))
    };
    let _: bool = msg_send![menu, popUpMenuPositioningItem: nil atLocation: pos inView: ns_view];

    let _: () = msg_send![handler, release];
    let _: () = msg_send![menu, release];

    match LIBRARY_MENU_SELECTION.load(Ordering::Relaxed) {
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
unsafe fn restore_main_view_first_responder(window: &Window) {
    let ns_view = match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as id,
            _ => return,
        },
        Err(_) => return,
    };
    let ns_window: id = msg_send![ns_view, window];
    if ns_window != nil {
        let _: bool = msg_send![ns_window, makeFirstResponder: ns_view];
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn nsstring_to_string(s: id) -> String {
    unsafe {
        let c_str: *const std::os::raw::c_char = msg_send![s, UTF8String];
        if c_str.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned()
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn install_macos_app_menu() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static mut HANDLER: id = nil;
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }

    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let main_menu: id = {
        let menu: id = msg_send![app, mainMenu];
        if menu == nil {
            let created: id = msg_send![class!(NSMenu), new];
            let _: () = msg_send![app, setMainMenu: created];
            created
        } else {
            menu
        }
    };

    let app_menu_item: id = {
        let count: i64 = msg_send![main_menu, numberOfItems];
        if count > 0 {
            msg_send![main_menu, itemAtIndex: 0i64]
        } else {
            let item: id = msg_send![class!(NSMenuItem), new];
            let _: () = msg_send![main_menu, addItem: item];
            item
        }
    };

    let process_info: id = msg_send![class!(NSProcessInfo), processInfo];
    let app_name: id = msg_send![process_info, processName];
    let _: () = msg_send![app_menu_item, setTitle: app_name];

    let app_menu: id = {
        let existing: id = msg_send![app_menu_item, submenu];
        if existing == nil {
            let created: id = msg_send![class!(NSMenu), new];
            let _: () = msg_send![app_menu_item, setSubmenu: created];
            created
        } else {
            existing
        }
    };

    // Remove any existing settings/preferences entry bound to Cmd+.
    let item_count: i64 = msg_send![app_menu, numberOfItems];
    for idx in (0..item_count).rev() {
        let item: id = msg_send![app_menu, itemAtIndex: idx];
        let key: id = msg_send![item, keyEquivalent];
        if nsstring_to_string(key) == "," {
            let _: () = msg_send![app_menu, removeItemAtIndex: idx];
        }
    }

    HANDLER = msg_send![menu_handler_class(), new];

    let pref_title = NSString::alloc(nil).init_str("Settings...");
    let pref_key = NSString::alloc(nil).init_str(",");
    let pref_item: id = msg_send![class!(NSMenuItem), alloc];
    let pref_item: id = msg_send![pref_item, initWithTitle: pref_title action: sel!(openPreferences:) keyEquivalent: pref_key];
    let _: () = msg_send![pref_item, setTarget: HANDLER];

    // Standard placement: after the About group separator.
    let count_after_cleanup: i64 = msg_send![app_menu, numberOfItems];
    let mut insert_index = if count_after_cleanup > 0 { 1i64 } else { 0i64 };
    for idx in 0..count_after_cleanup {
        let item: id = msg_send![app_menu, itemAtIndex: idx];
        let is_separator: bool = msg_send![item, isSeparatorItem];
        if is_separator {
            insert_index = idx + 1;
            break;
        }
    }
    let _: () = msg_send![app_menu, insertItem: pref_item atIndex: insert_index];
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
    decode_tx: Sender<DecodeResult>,
    /// Receiver half for background decode results.
    decode_rx: Receiver<DecodeResult>,
    /// Completed decode waiting for a non-blocking audio-engine handoff.
    pending_decode: Option<DecodeResult>,
    /// System info sampler (CPU / memory).
    sys: sysinfo::System,
    /// Last whole-second timestamp used for titlebar system-info refresh.
    last_titlebar_second: Option<u64>,
    /// Cached titlebar system fields that only need 1 Hz refresh.
    cached_titlebar: sujay_decks::ui_state::TitlebarState,
    /// Last full console snapshot submitted to the native renderer.
    last_console_visual: Option<sujay_decks::ui_state::ConsoleVisualState>,
    /// Last deck progress tuples submitted to the renderer: (pos, total, sr).
    last_deck_progress: [Option<(f32, f32, f32)>; 2],
    /// Timestamp when the current recording session started (None = not recording).
    rec_started_at: Option<Instant>,
    settings_path: PathBuf,
    preferences: AppPreferences,
    audio_devices: Vec<sujay_decks::ui_state::AudioDeviceInfo>,
    rekordbox_track_overrides: HashMap<PathBuf, RekordboxTrackOverride>,
    rekordbox_track_ids: HashMap<PathBuf, String>,
    rekordbox_master_db_path: Option<PathBuf>,
    rekordbox_load_tx: Sender<Result<RekordboxLibraryLoadReady, String>>,
    rekordbox_load_rx: Receiver<Result<RekordboxLibraryLoadReady, String>>,
    rekordbox_load_started: bool,
    rekordbox_load_in_flight: bool,
    rekordbox_db_modified: Option<SystemTime>,
    next_rekordbox_reload_check: Instant,
    deck_cues: [Vec<sujay_decks::ui_state::DeckCueVisualState>; 2],
}

impl SujayApp {
    fn new() -> Self {
        let (decode_tx, decode_rx) = mpsc::channel();
        let (rekordbox_load_tx, rekordbox_load_rx) = mpsc::channel();
        let mut sys = sysinfo::System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let pid = sysinfo::Pid::from_u32(std::process::id());
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);
        let settings_path = settings_file_path();
        let preferences = load_preferences(&settings_path);
        Self {
            window: None,
            engine: None,
            last_state: Arc::new(Mutex::new(None)),
            cursor_pos: (0.0, 0.0),
            hovered_deck: 1,
            decode_tx,
            decode_rx,
            pending_decode: None,
            sys,
            last_titlebar_second: None,
            cached_titlebar: sujay_decks::ui_state::TitlebarState::default(),
            last_console_visual: None,
            last_deck_progress: [None, None],
            rec_started_at: None,
            settings_path,
            preferences,
            audio_devices: vec![],
            rekordbox_track_overrides: HashMap::new(),
            rekordbox_track_ids: HashMap::new(),
            rekordbox_master_db_path: None,
            rekordbox_load_tx,
            rekordbox_load_rx,
            rekordbox_load_started: false,
            rekordbox_load_in_flight: false,
            rekordbox_db_modified: None,
            next_rekordbox_reload_check: Instant::now(),
            deck_cues: [Vec::new(), Vec::new()],
        }
    }

    fn dispatch_action(&mut self, action: sujay_decks::UiAction) {
        use sujay_decks::UiAction;

        match action {
            UiAction::LoadFile(deck, path) => {
                let path_buf = PathBuf::from(path);
                #[cfg(target_os = "macos")]
                unsafe {
                    if let Some(window) = self.window.as_ref() {
                        restore_main_view_first_responder(window);
                    }
                }
                let (override_meta, content_id) = if let (Some(ov), Some(id)) = (
                    self.rekordbox_track_overrides.get(&path_buf),
                    self.rekordbox_track_ids.get(&path_buf),
                ) {
                    (Some(ov.clone()), Some(id.clone()))
                } else {
                    let key = normalize_track_path(&path_buf);
                    (
                        self.rekordbox_track_overrides.get(&key).cloned(),
                        self.rekordbox_track_ids.get(&key).cloned(),
                    )
                };
                eprintln!(
                    "[Library] LoadFile deck={} path={} override={} content_id={}",
                    deck,
                    path_buf.display(),
                    override_meta.is_some(),
                    content_id.as_deref().unwrap_or("-"),
                );
                spawn_decode(
                    deck,
                    path_buf,
                    self.decode_tx.clone(),
                    override_meta,
                    self.rekordbox_master_db_path.clone(),
                    content_id,
                );
                return;
            }
            UiAction::OpenPreferences => {
                self.open_native_preferences_dialog();
                return;
            }
            UiAction::OpenLibraryContextMenu(path, x, y) => {
                #[cfg(target_os = "macos")]
                unsafe {
                    if let Some(window) = self.window.as_ref() {
                        if let Some(deck) = show_native_library_context_menu(window, x, y) {
                            eprintln!(
                                "[Library] native context menu selected deck={} path={}",
                                deck, path
                            );
                            self.dispatch_action(UiAction::LoadFile(deck, path));
                        } else {
                            eprintln!("[Library] native context menu dismissed");
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = (path, x, y);
                return;
            }
            _ => {}
        }

        let Some(engine) = self.engine.as_ref().cloned() else {
            return;
        };

        match action {
            UiAction::Play(deck) => {
                let _ = engine.play(deck as u32);
            }
            UiAction::Stop(deck) => {
                let _ = engine.stop(deck as u32);
            }
            UiAction::SetCrossfader(v) => {
                let _ = engine.set_crossfader_position(v as f64);
            }
            UiAction::SetMasterTempo(v) => {
                let _ = engine.set_master_tempo(v as f64);
            }
            UiAction::SetDeckGain(deck, v) => {
                let _ = engine.set_deck_gain(deck as u32, v as f64);
            }
            UiAction::SetCue(deck, enabled) => {
                let _ = engine.set_deck_cue_enabled(deck as u32, enabled);
            }
            UiAction::SetEq(deck, band, kill) => {
                let _ = engine.set_eq_cut(deck as u32, band, kill);
            }
            UiAction::Seek(deck, pos) => {
                let _ = engine.seek(deck as u32, pos as f64);
            }
            UiAction::RecallCue(deck, position, loop_end) => {
                let _ = engine.seek(deck as u32, position as f64);
                if let Some(loop_end) = loop_end {
                    let _ = engine.set_loop(deck as u32, position as f64, loop_end as f64, true);
                } else {
                    let _ = engine.clear_loop(deck as u32);
                }
            }
            UiAction::ToggleLoop(deck, beats) => {
                if beats <= 0.0 {
                    let _ = engine.clear_loop(deck as u32);
                } else if let Some((beat_grid, current_pos)) =
                    sujay_decks::get_deck_beat_info_raw(deck as u32)
                {
                    let start_beat_idx = beat_grid
                        .partition_point(|&b| b <= current_pos)
                        .saturating_sub(1);
                    let start_frames = beat_grid
                        .get(start_beat_idx)
                        .copied()
                        .unwrap_or(current_pos);
                    let beats_whole = beats.floor() as usize;
                    let beats_frac = beats - beats.floor();
                    let end_frames = if beats_frac < 0.001 {
                        let end_idx = start_beat_idx + beats_whole;
                        if end_idx < beat_grid.len() {
                            beat_grid[end_idx]
                        } else {
                            let beat_interval = if beat_grid.len() >= 2 {
                                beat_grid[beat_grid.len() - 1] - beat_grid[beat_grid.len() - 2]
                            } else {
                                engine.sample_rate as f32 * 60.0 / 120.0
                            };
                            start_frames + beat_interval * beats_whole as f32
                        }
                    } else {
                        let beat_interval = if start_beat_idx + 1 < beat_grid.len() {
                            beat_grid[start_beat_idx + 1] - start_frames
                        } else if beat_grid.len() >= 2 {
                            beat_grid[beat_grid.len() - 1] - beat_grid[beat_grid.len() - 2]
                        } else {
                            engine.sample_rate as f32 * 60.0 / 120.0
                        };
                        start_frames + beat_interval * beats
                    };

                    let sr = engine.sample_rate as f64;
                    let _ = engine.set_beat_loop(
                        deck as u32,
                        start_frames as f64 / sr,
                        end_frames as f64 / sr,
                    );
                } else {
                    let _ = engine.toggle_beat_loop(deck as u32, beats);
                }
            }
            UiAction::SetMicEnabled(enabled) => {
                let _ = engine.set_mic_enabled(enabled);
            }
            UiAction::StartRecording => match prepare_recording_path(&self.preferences) {
                Ok(path) => {
                    if let Err(err) = engine.start_recording(
                        path.to_string_lossy().to_string(),
                        &self.preferences.recording_format,
                    ) {
                        tracing::warn!("failed to start recording: {}", err);
                    }
                }
                Err(err) => {
                    tracing::warn!("failed to prepare recording path: {}", err);
                }
            },
            UiAction::StopRecording => {
                let _ = engine.stop_recording();
            }
            UiAction::SavePreferences(state) => {
                apply_preferences_state(&mut self.preferences, state, &self.audio_devices);

                if let Err(err) = save_preferences(&self.settings_path, &self.preferences) {
                    tracing::warn!("failed to save preferences: {}", err);
                }
                if let Err(err) =
                    engine.configure_device(device_config_from_preferences(&self.preferences))
                {
                    tracing::warn!("failed to apply audio preferences: {}", err);
                }
                set_preferences_state_raw(ui_preferences_state(
                    &self.preferences,
                    &self.audio_devices,
                ));
            }
            UiAction::LoadFile(_, _)
            | UiAction::OpenPreferences
            | UiAction::OpenLibraryContextMenu(_, _, _) => {}
        }
    }

    fn open_native_preferences_dialog(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            let current = ui_preferences_state(&self.preferences, &self.audio_devices);
            if let Some(next) = mac_settings::show_native_preferences_dialog(&current) {
                apply_preferences_state(&mut self.preferences, next, &self.audio_devices);

                if let Err(err) = save_preferences(&self.settings_path, &self.preferences) {
                    tracing::warn!("failed to save preferences: {}", err);
                }
                if let Some(engine) = self.engine.as_ref() {
                    if let Err(err) =
                        engine.configure_device(device_config_from_preferences(&self.preferences))
                    {
                        tracing::warn!("failed to apply audio preferences: {}", err);
                    }
                }
                set_preferences_state_raw(ui_preferences_state(
                    &self.preferences,
                    &self.audio_devices,
                ));
            }
        }
    }
}

/// Native Windows menu bar with a "Sujay → Settings…" entry. The window proc
/// is subclassed so the menu command opens the egui preferences modal.
#[cfg(target_os = "windows")]
mod win_menu {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CallWindowProcW, CreateMenu, CreatePopupMenu, DrawMenuBar, MessageBoxW,
        PostMessageW, SetMenu, SetWindowLongPtrW, GWLP_WNDPROC, MB_OK, MF_POPUP, MF_SEPARATOR,
        MF_STRING, WM_CLOSE, WM_COMMAND, WNDPROC,
    };

    const ID_SETTINGS: usize = 0xA001;
    const ID_EXIT: usize = 0xA002;
    const ID_ABOUT: usize = 0xA003;
    static OLD_WNDPROC: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_COMMAND {
            match (wparam & 0xFFFF) as usize {
                ID_SETTINGS => {
                    crate::win_settings::request_open();
                    return 0;
                }
                ID_EXIT => {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    return 0;
                }
                ID_ABOUT => {
                    MessageBoxW(
                        hwnd,
                        wide("Sujay — AI DJ").as_ptr(),
                        wide("About Sujay").as_ptr(),
                        MB_OK,
                    );
                    return 0;
                }
                _ => {}
            }
        }
        let old: WNDPROC =
            std::mem::transmute::<isize, WNDPROC>(OLD_WNDPROC.load(Ordering::Relaxed));
        CallWindowProcW(old, hwnd, msg, wparam, lparam)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub unsafe fn install(hwnd: HWND) {
        let menubar = CreateMenu();

        // File → Settings… / Exit
        let file = CreatePopupMenu();
        AppendMenuW(file, MF_STRING, ID_SETTINGS, wide("Settings...").as_ptr());
        AppendMenuW(file, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(file, MF_STRING, ID_EXIT, wide("Exit").as_ptr());
        AppendMenuW(menubar, MF_POPUP, file as usize, wide("File").as_ptr());

        // Help → About Sujay
        let help = CreatePopupMenu();
        AppendMenuW(help, MF_STRING, ID_ABOUT, wide("About Sujay").as_ptr());
        AppendMenuW(menubar, MF_POPUP, help as usize, wide("Help").as_ptr());

        SetMenu(hwnd, menubar);
        DrawMenuBar(hwnd);

        let new_proc: WNDPROC = Some(wndproc);
        let old = SetWindowLongPtrW(
            hwnd,
            GWLP_WNDPROC,
            std::mem::transmute::<WNDPROC, isize>(new_proc),
        );
        OLD_WNDPROC.store(old, Ordering::Relaxed);
    }
}

impl ApplicationHandler for SujayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Sujay")
            .with_inner_size(winit::dpi::LogicalSize::new(1100u32, 760u32));

        // macOS: transparent titlebar so our egui titlebar replaces it,
        // while native traffic-light buttons remain.
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        };

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let scale = window.scale_factor();
        // Windows re-reads the client size after installing its menu bar (which
        // shrinks the client area), so this outer binding is only used on macOS.
        #[cfg(target_os = "macos")]
        let logical = window.inner_size().to_logical::<f64>(scale);

        // ── Attach native UI renderer ────────────────────────────────────────
        #[cfg(target_os = "macos")]
        {
            unsafe {
                install_macos_app_menu();
            }
            let ns_view = match window.window_handle().unwrap().as_raw() {
                RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut std::ffi::c_void,
                _ => panic!("unexpected window handle type"),
            };
            attach_raw(ns_view, 0.0, 0.0, logical.width, logical.height);
        }

        #[cfg(target_os = "windows")]
        {
            let hwnd = match window.window_handle().unwrap().as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get() as *mut std::ffi::c_void,
                _ => panic!("unexpected window handle type"),
            };
            unsafe {
                win_menu::install(hwnd as windows_sys::Win32::Foundation::HWND);
            }
            // The menu bar consumes client height; re-read the now-smaller client size.
            let logical = window.inner_size().to_logical::<f64>(scale);
            attach_raw(hwnd, 0.0, 0.0, logical.width, logical.height);
        }

        // ── Start audio engine ───────────────────────────────────────────────
        let last_state = Arc::clone(&self.last_state);
        let engine = Arc::new(
            AudioEngineCore::new(
                Some(44100),
                Arc::new(move |state: EngineStateUpdate| {
                    if let Ok(mut guard) = last_state.lock() {
                        *guard = Some(state);
                    }
                }),
            )
            .expect("Failed to initialise audio engine"),
        );

        self.audio_devices = available_audio_devices();
        normalize_preferences(&mut self.preferences, &self.audio_devices);
        if let Err(err) = engine.configure_device(device_config_from_preferences(&self.preferences))
        {
            tracing::warn!("failed to configure initial audio device: {}", err);
        }
        set_preferences_state_raw(ui_preferences_state(&self.preferences, &self.audio_devices));

        self.start_rekordbox_library_load();

        self.window = Some(window);
        self.engine = Some(engine);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("window close requested");
                if let Some(engine) = self.engine.take() {
                    engine.close();
                }
                detach_raw();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let scale = self
                    .window
                    .as_ref()
                    .map(|w| w.scale_factor())
                    .unwrap_or(1.0);
                let logical = size.to_logical::<f64>(scale);
                set_frame_raw(0.0, 0.0, logical.width, logical.height);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map(|w| w.scale_factor())
                    .unwrap_or(1.0);
                let logical = position.to_logical::<f32>(scale);
                self.cursor_pos = (logical.x, logical.y);
                #[cfg(not(target_os = "macos"))]
                push_mouse_event_raw(0, logical.x, logical.y);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                #[cfg(not(target_os = "macos"))]
                let kind = match (button, state) {
                    (winit::event::MouseButton::Left, winit::event::ElementState::Pressed) => {
                        Some(1)
                    }
                    (winit::event::MouseButton::Left, winit::event::ElementState::Released) => {
                        Some(2)
                    }
                    (winit::event::MouseButton::Right, winit::event::ElementState::Pressed) => {
                        Some(3)
                    }
                    (winit::event::MouseButton::Right, winit::event::ElementState::Released) => {
                        Some(4)
                    }
                    _ => None,
                };
                #[cfg(not(target_os = "macos"))]
                if let Some(kind) = kind {
                    push_mouse_event_raw(kind, self.cursor_pos.0, self.cursor_pos.1);
                }

                // Drag only on left press in the titlebar middle area.
                if button == winit::event::MouseButton::Left
                    && state == winit::event::ElementState::Pressed
                    && self.cursor_pos.1 < 38.0
                {
                    let win_w = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size().to_logical::<f32>(w.scale_factor()).width)
                        .unwrap_or(1100.0);
                    let in_drag_area =
                        self.cursor_pos.0 > 80.0 && self.cursor_pos.0 < win_w - 360.0;
                    if in_drag_area {
                        if let Some(ref win) = self.window {
                            let _ = win.drag_window();
                        }
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            WindowEvent::MouseWheel { delta, .. } => {
                let wheel_points = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 48.0,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        let scale = self
                            .window
                            .as_ref()
                            .map(|w| w.scale_factor())
                            .unwrap_or(1.0);
                        pos.to_logical::<f32>(scale).y * 2.0
                    }
                };
                push_mouse_event_raw(5, 0.0, wheel_points);
            }
            #[cfg(target_os = "macos")]
            WindowEvent::MouseWheel { .. } => {}
            // Track hovered file position so we know which deck the user is aiming at
            WindowEvent::HoveredFile(_) => {
                let win_width = self
                    .window
                    .as_ref()
                    .map(|w| {
                        let scale = w.scale_factor();
                        w.inner_size().to_logical::<f32>(scale).width
                    })
                    .unwrap_or(1100.0);
                self.hovered_deck = if self.cursor_pos.0 < win_width * 0.5 {
                    1
                } else {
                    2
                };
                eprintln!(
                    "[D&D] hover x={:.0} width={:.0} -> deck {}",
                    self.cursor_pos.0, win_width, self.hovered_deck
                );
            }
            // winit drag-and-drop (fires when SujayMouseView doesn't handle the drop)
            WindowEvent::DroppedFile(path) => {
                eprintln!("[D&D] DroppedFile {:?} -> deck {}", path, self.hovered_deck);
                let (override_meta, content_id) = if let (Some(ov), Some(id)) = (
                    self.rekordbox_track_overrides.get(&path),
                    self.rekordbox_track_ids.get(&path),
                ) {
                    (Some(ov.clone()), Some(id.clone()))
                } else {
                    let key = normalize_track_path(&path);
                    (
                        self.rekordbox_track_overrides.get(&key).cloned(),
                        self.rekordbox_track_ids.get(&key).cloned(),
                    )
                };
                spawn_decode(
                    self.hovered_deck,
                    path,
                    self.decode_tx.clone(),
                    override_meta,
                    self.rekordbox_master_db_path.clone(),
                    content_id,
                );
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut needs_redraw = false;

        #[cfg(target_os = "macos")]
        if PREFS_REQUESTED.swap(false, Ordering::Relaxed) {
            self.open_native_preferences_dialog();
            needs_redraw = true;
        }

        #[cfg(target_os = "windows")]
        {
            if win_settings::take_open_request() {
                self.audio_devices = available_audio_devices();
                normalize_preferences(&mut self.preferences, &self.audio_devices);
                let current = ui_preferences_state(&self.preferences, &self.audio_devices);
                if let Some(window) = self.window.as_ref() {
                    if let RawWindowHandle::Win32(h) = window.window_handle().unwrap().as_raw() {
                        unsafe {
                            win_settings::open(h.hwnd.get() as _, &current);
                        }
                    }
                }
                needs_redraw = true;
            }
            if let Some(next) = win_settings::take_result() {
                apply_preferences_state(&mut self.preferences, next, &self.audio_devices);
                if let Err(err) = save_preferences(&self.settings_path, &self.preferences) {
                    tracing::warn!("failed to save preferences: {}", err);
                }
                if let Some(engine) = self.engine.as_ref() {
                    if let Err(err) =
                        engine.configure_device(device_config_from_preferences(&self.preferences))
                    {
                        tracing::warn!("failed to apply audio preferences: {}", err);
                    }
                }
                set_preferences_state_raw(ui_preferences_state(
                    &self.preferences,
                    &self.audio_devices,
                ));
                needs_redraw = true;
            }
        }

        // Drain UI actions every frame. Some actions (e.g. library load) are
        // valid before/without an initialized audio engine.
        let mut handled_any_action = false;
        for action in poll_actions_raw() {
            self.dispatch_action(action);
            handled_any_action = true;
        }
        needs_redraw |= handled_any_action;

        // Apply background Rekordbox load result without blocking the main thread.
        while let Ok(result) = self.rekordbox_load_rx.try_recv() {
            let had_library = self.rekordbox_master_db_path.is_some();
            self.rekordbox_load_in_flight = false;
            match result {
                Ok(ready) => {
                    eprintln!(
                        "[Rekordbox] loaded library: tracks={} source={}",
                        ready.tracks.len(),
                        ready.source_label
                    );
                    for track in ready.tracks.iter().take(5) {
                        eprintln!(
                            "[Rekordbox] track title={:?} artist={:?} path={}",
                            track.title, track.artist, track.file_path
                        );
                    }
                    self.rekordbox_db_modified = fs::metadata(&ready.master_db_path)
                        .and_then(|metadata| metadata.modified())
                        .ok();
                    self.rekordbox_master_db_path = Some(ready.master_db_path);
                    self.rekordbox_track_overrides = ready.overrides;
                    self.rekordbox_track_ids = ready.track_ids;
                    set_library_state_raw(sujay_decks::ui_state::LibraryVisualState {
                        source_label: ready.source_label,
                        tracks: ready.tracks,
                        playlists: ready.playlists,
                    });
                }
                Err(err) => {
                    tracing::warn!("failed to load rekordbox library: {}", err);
                    if !had_library {
                        self.rekordbox_master_db_path = None;
                        self.rekordbox_track_overrides.clear();
                        self.rekordbox_track_ids.clear();
                        set_library_state_raw(sujay_decks::ui_state::LibraryVisualState {
                            source_label: "Rekordbox library not found".to_owned(),
                            tracks: vec![],
                            playlists: vec![],
                        });
                    }
                }
            }
            needs_redraw = true;
        }
        self.poll_rekordbox_library_reload();

        // Push latest engine state into the UI renderer
        if let Ok(mut guard) = self.last_state.lock() {
            if let Some(state) = guard.take() {
                // Track recording start time
                if state.is_recording && self.rec_started_at.is_none() {
                    self.rec_started_at = Some(Instant::now());
                } else if !state.is_recording {
                    self.rec_started_at = None;
                }

                use std::time::{SystemTime, UNIX_EPOCH};
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if self.last_titlebar_second != Some(now_secs) {
                    self.last_titlebar_second = Some(now_secs);
                    self.sys.refresh_cpu_all();
                    self.sys.refresh_memory();
                    let pid = sysinfo::Pid::from_u32(std::process::id());
                    self.sys
                        .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

                    self.cached_titlebar.time_text = {
                        #[cfg(target_os = "macos")]
                        {
                            let ts = now_secs as libc::time_t;
                            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                            unsafe {
                                libc::localtime_r(&ts, &mut tm);
                            }
                            format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            let h = (now_secs % 86400) / 3600;
                            let m = (now_secs % 3600) / 60;
                            let s = now_secs % 60;
                            format!("{:02}:{:02}:{:02}", h, m, s)
                        }
                    };
                    self.cached_titlebar.cpu_percent = self.sys.global_cpu_usage();
                    self.cached_titlebar.mem_mb = self
                        .sys
                        .process(pid)
                        .map(|p| p.memory() / 1024 / 1024)
                        .unwrap_or(0);
                }

                let rec_elapsed_secs = self
                    .rec_started_at
                    .map(|t| t.elapsed().as_secs() as u32)
                    .unwrap_or(0);

                let mut cv = engine_state_to_console_visual(&state);
                cv.deck_a.rekordbox_cues = self.deck_cues[0].clone();
                cv.deck_b.rekordbox_cues = self.deck_cues[1].clone();
                cv.titlebar = sujay_decks::ui_state::TitlebarState {
                    time_text: self.cached_titlebar.time_text.clone(),
                    cpu_percent: self.cached_titlebar.cpu_percent,
                    mem_mb: self.cached_titlebar.mem_mb,
                    mic_available: state.mic_available,
                    mic_enabled: state.mic_enabled,
                    mic_peak: state.mic_peak as f32,
                    is_recording: state.is_recording,
                    rec_elapsed_secs,
                };
                if self.last_console_visual.as_ref() != Some(&cv) {
                    self.last_console_visual = Some(cv.clone());
                    set_console_state_raw(cv);
                    needs_redraw = true;
                }

                let sr = state.sample_rate as f32;
                if let Some(pos) = state.deck_a_position {
                    let next = (
                        pos as f32,
                        state.deck_a_total_frames.unwrap_or(0.0) as f32,
                        sr,
                    );
                    if self.last_deck_progress[0] != Some(next) {
                        self.last_deck_progress[0] = Some(next);
                        set_deck_progress_raw(1, next.0, next.1, next.2);
                        needs_redraw = true;
                    }
                }
                if let Some(pos) = state.deck_b_position {
                    let next = (
                        pos as f32,
                        state.deck_b_total_frames.unwrap_or(0.0) as f32,
                        sr,
                    );
                    if self.last_deck_progress[1] != Some(next) {
                        self.last_deck_progress[1] = Some(next);
                        set_deck_progress_raw(2, next.0, next.1, next.2);
                        needs_redraw = true;
                    }
                }
            }
        }

        // Drain completed background decodes and load into engine on main thread
        if let Some(engine) = self.engine.clone() {
            // Keep the UI/event loop responsive even if multiple decode jobs complete together.
            if let Some(result) = self
                .pending_decode
                .take()
                .or_else(|| self.decode_rx.try_recv().ok())
            {
                needs_redraw = true;
                match result {
                    DecodeResult::Ready(ready) => {
                        let DecodeReady {
                            deck,
                            pcm,
                            waveform,
                            waveform_colors,
                            bpm,
                            title,
                            beats,
                            intro,
                            outro,
                            cues,
                            total_frames,
                        } = ready;
                        eprintln!(
                            "[D&D] Loading deck {} title={:?} bpm={:?}",
                            deck, title, bpm
                        );
                        let mut pcm = Some(pcm);
                        let mut track_id = Some(title.clone());
                        if !engine.try_load_track(
                            deck as u32,
                            &mut pcm,
                            bpm,
                            beats.clone(),
                            &mut track_id,
                        ) {
                            self.pending_decode = Some(DecodeResult::Ready(DecodeReady {
                                deck,
                                pcm: pcm.expect("PCM retained when audio engine is busy"),
                                waveform,
                                waveform_colors,
                                bpm,
                                title,
                                beats,
                                intro,
                                outro,
                                cues,
                                total_frames,
                            }));
                            event_loop.set_control_flow(ControlFlow::WaitUntil(
                                Instant::now() + Duration::from_millis(1),
                            ));
                            return;
                        }
                        let sr = engine.sample_rate as f32;
                        if let Some(deck_cues) =
                            self.deck_cues.get_mut((deck as usize).wrapping_sub(1))
                        {
                            *deck_cues = cues;
                        }
                        sujay_decks::set_waveform_raw(deck as u32, waveform);
                        sujay_decks::set_waveform_colors_raw(deck as u32, waveform_colors);
                        {
                            let first_beat = beats.first().copied();
                            let last_beat = beats.last().copied();
                            if let (Some(fb), Some(lb)) = (first_beat, last_beat) {
                                eprintln!(
                                    "[DEBUG] deck={} sr={} total_frames={} beats[0]={:.0} beats[-1]={:.0} \
                                     beat[0]_sec={:.2} beat[-1]_sec={:.2}",
                                    deck, sr, total_frames, fb, lb, fb / sr, lb / sr,
                                );
                            }
                        }
                        sujay_decks::set_deck_markers_raw(deck as u32, beats, intro, outro);
                        sujay_decks::set_deck_progress_raw(deck as u32, 0.0, total_frames, sr);
                    }
                    DecodeResult::Failed(failure) => {
                        let msg = format!("{}", failure.error);
                        eprintln!(
                            "[D&D] Decode failed for deck={} path={} reason={}",
                            failure.deck,
                            failure.path.display(),
                            msg
                        );
                    }
                }
            }
        }

        // Wake at the audio-state cadence instead of spinning the main thread.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(33),
        ));

        if needs_redraw {
            if let Some(ref w) = self.window {
                w.request_redraw();
            }
        }
    }
}

impl SujayApp {
    fn start_rekordbox_library_load(&mut self) {
        if self.rekordbox_load_in_flight {
            return;
        }
        self.rekordbox_load_in_flight = true;

        set_library_state_raw(sujay_decks::ui_state::LibraryVisualState {
            source_label: "Loading Rekordbox library...".to_owned(),
            tracks: vec![],
            playlists: vec![],
        });

        spawn_rekordbox_library_load(self.rekordbox_load_tx.clone());
    }

    fn poll_rekordbox_library_reload(&mut self) {
        if self.rekordbox_load_in_flight || Instant::now() < self.next_rekordbox_reload_check {
            return;
        }
        self.next_rekordbox_reload_check = Instant::now() + Duration::from_secs(2);

        let Some(master_db_path) = self.rekordbox_master_db_path.as_ref() else {
            return;
        };
        let modified = fs::metadata(master_db_path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if modified.is_some() && modified != self.rekordbox_db_modified {
            eprintln!(
                "[Rekordbox] master.db changed; refreshing library from {}",
                master_db_path.display()
            );
            self.rekordbox_load_in_flight = true;
            spawn_rekordbox_library_load(self.rekordbox_load_tx.clone());
        }
    }
}

fn spawn_rekordbox_library_load(tx: Sender<Result<RekordboxLibraryLoadReady, String>>) {
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
                    .map(|playlist| sujay_decks::ui_state::LibraryPlaylistItem {
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

                    tracks.push(sujay_decks::ui_state::LibraryTrackItem {
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

/// Decode `path` on a background thread and send the result via `tx`.
/// Decode a source file and resolve its metadata (BPM, beats, waveform), applying
/// any rekordbox override.  Shared by deck loading and AI extension.  Runs the
/// decode under `catch_unwind` so a codec panic becomes an `Err`.
fn decode_track_full(
    path: &Path,
    override_meta: Option<RekordboxTrackOverride>,
) -> Result<DecodedTrack, String> {
    let path_str = path.to_string_lossy().to_string();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sujay_audio::decoder::decode_audio(path_str.clone(), 44100, 2)
    }));
    let result = match res {
        Err(e) => {
            let msg = e
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| e.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("(unknown panic)");
            return Err(format!("Decode panicked: {msg}"));
        }
        Ok(Err(e)) => return Err(e.to_string()),
        Ok(Ok(result)) => result,
    };

    let sr = result.sample_rate as f32;
    let mut bpm = result.bpm.map(|b| b as f32);
    // 44100 Hz → ~200 Hz (step=220): 5min track ≈ 60k points.
    let step = (result.sample_rate as usize / 200).max(1);
    let mut waveform: Vec<f32> = result
        .mono
        .chunks(step)
        .map(|chunk| chunk.iter().map(|&s| s.abs()).fold(0.0f32, f32::max))
        .collect();
    let mut waveform_colors = Vec::new();
    let mut rekordbox_cues = Vec::new();
    let mut title = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Convert beat/intro/outro from seconds → audio frames
    let (mut beats, mut intro, mut outro) = if let Some(ref st) = result.structure {
        let beats = st.beats.iter().map(|&s| s as f32 * sr).collect();
        let intro = Some(st.intro.end as f32 * sr);
        let outro = Some(st.outro.start as f32 * sr);
        (beats, intro, outro)
    } else {
        (vec![], None, None)
    };

    if let Some(meta) = override_meta {
        if !meta.title.is_empty() {
            title = meta.title;
        }
        if meta.bpm.is_some() {
            bpm = meta.bpm;
        }
        if !meta.waveform.is_empty() {
            waveform = meta.waveform;
        }
        if !meta.waveform_colors.is_empty() {
            waveform_colors = meta.waveform_colors;
        }
        if !meta.beats_ms.is_empty() {
            beats = meta.beats_ms.iter().map(|ms| (*ms / 1000.0) * sr).collect();
            intro = None;
            outro = None;
        }
        rekordbox_cues = meta.cues;
    }
    let total_frames = result.pcm.len() / 2;
    let cues = rekordbox_cues_to_visuals(&rekordbox_cues, total_frames, sr);

    Ok(DecodedTrack {
        pcm: result.pcm,
        sample_rate: result.sample_rate,
        bpm,
        beats,
        intro,
        outro,
        waveform,
        waveform_colors,
        cues,
        title,
    })
}

fn spawn_decode(
    deck: u8,
    path: PathBuf,
    tx: Sender<DecodeResult>,
    mut override_meta: Option<RekordboxTrackOverride>,
    master_db_path: Option<PathBuf>,
    rekordbox_content_id: Option<String>,
) {
    std::thread::spawn(move || {
        eprintln!("[D&D] Decoding {} for deck {}", path.display(), deck);
        if let (Some(meta), Some(master_db_path), Some(content_id)) = (
            override_meta.as_mut(),
            master_db_path.as_deref(),
            rekordbox_content_id.as_deref(),
        ) {
            for attempt in 1..=3 {
                match sujay_library::load_track_beats_from_master_db(master_db_path, content_id) {
                    Ok(beats_ms) => {
                        eprintln!(
                            "[Rekordbox] loaded beat grid for {}: {} markers",
                            path.display(),
                            beats_ms.len()
                        );
                        meta.beats_ms = beats_ms;
                        break;
                    }
                    Err(err) if attempt < 3 => {
                        eprintln!(
                            "[Rekordbox] beat grid read failed for {} (attempt {attempt}/3): {err}",
                            path.display()
                        );
                        std::thread::sleep(Duration::from_millis(100 * attempt));
                    }
                    Err(err) => eprintln!(
                        "[Rekordbox] beat grid unavailable for {} after {attempt} attempts: {err}",
                        path.display()
                    ),
                }
            }
            match sujay_library::load_track_cues_from_master_db(master_db_path, content_id) {
                Ok(cues) => {
                    eprintln!(
                        "[Rekordbox] loaded {} cue entries for {}",
                        cues.len(),
                        path.display()
                    );
                    meta.cues = cues;
                }
                Err(err) => eprintln!(
                    "[Rekordbox] cue read unavailable for {}: {err}",
                    path.display()
                ),
            }
        }
        match decode_track_full(&path, override_meta) {
            Ok(t) => {
                eprintln!(
                    "[D&D] Decode done deck={} bpm={:?} beats={} title={:?}",
                    deck,
                    t.bpm,
                    t.beats.len(),
                    t.title
                );
                let total_frames = (t.pcm.len() / 2) as f32;
                let _ = tx.send(DecodeResult::Ready(DecodeReady {
                    deck,
                    pcm: t.pcm,
                    waveform: t.waveform,
                    waveform_colors: t.waveform_colors,
                    bpm: t.bpm,
                    title: t.title,
                    beats: t.beats,
                    intro: t.intro,
                    outro: t.outro,
                    cues: t.cues,
                    total_frames,
                }));
            }
            Err(error) => {
                eprintln!("[D&D] Decode failed for deck={}: {}", deck, error);
                let _ = tx.send(DecodeResult::Failed(DecodeFailure { deck, path, error }));
            }
        }
    });
}

fn rekordbox_cues_to_visuals(
    cues: &[sujay_library::RekordboxCue],
    total_frames: usize,
    sample_rate: f32,
) -> Vec<sujay_decks::ui_state::DeckCueVisualState> {
    let duration_ms = total_frames as f32 / sample_rate.max(1.0) * 1000.0;
    if duration_ms <= 0.0 {
        return Vec::new();
    }

    let mut memory_cue_index = 0;
    cues.iter()
        .filter_map(|cue| {
            let position = cue.time_ms as f32 / duration_ms;
            if !(0.0..=1.0).contains(&position) {
                return None;
            }
            let loop_end = if cue.is_loop && cue.loop_time_ms > cue.time_ms {
                let end = cue.loop_time_ms as f32 / duration_ms;
                (end <= 1.0).then_some(end)
            } else {
                None
            };
            let label = if cue.hot_cue > 0 {
                cue.hot_cue.to_string()
            } else {
                memory_cue_index += 1;
                format!("M{memory_cue_index}")
            };
            Some(sujay_decks::ui_state::DeckCueVisualState {
                label,
                position,
                loop_end,
                color_rgb: cue.color_rgb.map(|(red, green, blue)| [red, green, blue]),
            })
        })
        .collect()
}

fn normalize_track_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

// ── Engine state → UI visual state mapping ───────────────────────────────────

fn engine_state_to_console_visual(
    s: &sujay_audio::engine_core::EngineStateUpdate,
) -> sujay_decks::ui_state::ConsoleVisualState {
    use sujay_decks::ui_state::{ConsoleVisualState, DeckConsoleVisualState};

    fn fmt_time(frames: f64, sr: f64) -> String {
        if sr == 0.0 {
            return "0:00".into();
        }
        let secs = (frames / sr) as u64;
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    let sr = s.sample_rate;

    // Compute loop_beats from loop length and track BPM so the correct button
    // appears highlighted.  Falls back to 0.0 (no active button) if unknown.
    let calc_loop_beats = |loop_enabled: bool, start: f64, end: f64, bpm: Option<f64>| -> f32 {
        if !loop_enabled || bpm.is_none() || bpm == Some(0.0) {
            return 0.0;
        }
        let beat_interval = sr * 60.0 / bpm.unwrap();
        let beats = (end - start) / beat_interval;
        // Round to nearest standard value
        let standards = [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
        standards
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - beats as f32)
                    .abs()
                    .partial_cmp(&(b - beats as f32).abs())
                    .unwrap()
            })
            .unwrap_or(0.0)
    };

    let deck_a = DeckConsoleVisualState {
        title: s.deck_a_track_id.clone().unwrap_or_else(|| "---".into()),
        time_text: s
            .deck_a_position
            .map(|p| fmt_time(p, sr))
            .unwrap_or_else(|| "0:00".into()),
        bpm_text: s
            .deck_a_bpm
            .map(|b| format!("{:.1}", b))
            .unwrap_or_else(|| "--.-".into()),
        bpm: s.deck_a_bpm.unwrap_or(s.master_tempo) as f32,
        playing: s.deck_a_playing,
        loop_enabled: s.deck_a_loop.enabled,
        loop_beats: calc_loop_beats(
            s.deck_a_loop.enabled,
            s.deck_a_loop.start,
            s.deck_a_loop.end,
            s.deck_a_bpm,
        ),
        loop_start: s.deck_a_loop.start as f32,
        loop_end: s.deck_a_loop.end as f32,
        cue_enabled: s.deck_a_cue_enabled,
        eq_low: s.deck_a_eq_cut.low,
        eq_mid: s.deck_a_eq_cut.mid,
        eq_high: s.deck_a_eq_cut.high,
        gain: s.deck_a_gain as f32,
        peak: s.deck_a_peak as f32,
        rekordbox_cues: Vec::new(),
    };
    let deck_b = DeckConsoleVisualState {
        title: s.deck_b_track_id.clone().unwrap_or_else(|| "---".into()),
        time_text: s
            .deck_b_position
            .map(|p| fmt_time(p, sr))
            .unwrap_or_else(|| "0:00".into()),
        bpm_text: s
            .deck_b_bpm
            .map(|b| format!("{:.1}", b))
            .unwrap_or_else(|| "--.-".into()),
        bpm: s.deck_b_bpm.unwrap_or(s.master_tempo) as f32,
        playing: s.deck_b_playing,
        loop_enabled: s.deck_b_loop.enabled,
        loop_beats: calc_loop_beats(
            s.deck_b_loop.enabled,
            s.deck_b_loop.start,
            s.deck_b_loop.end,
            s.deck_b_bpm,
        ),
        loop_start: s.deck_b_loop.start as f32,
        loop_end: s.deck_b_loop.end as f32,
        cue_enabled: s.deck_b_cue_enabled,
        eq_low: s.deck_b_eq_cut.low,
        eq_mid: s.deck_b_eq_cut.mid,
        eq_high: s.deck_b_eq_cut.high,
        gain: s.deck_b_gain as f32,
        peak: s.deck_b_peak as f32,
        rekordbox_cues: Vec::new(),
    };
    ConsoleVisualState {
        titlebar: Default::default(), // overwritten in about_to_wait with live data
        deck_a,
        deck_b,
        master_tempo: s.master_tempo as f32,
        crossfader: s.crossfader_position as f32,
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Sujay starting (Phase 5 Rust-native)");

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = SujayApp::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
