#![allow(unexpected_cfgs)]

//! Sujay — Phase 5 Rust-native host binary.
//!
//! Creates a winit window, attaches the wgpu/egui UI renderer, and drives the
//! Rust AudioEngineCore — no Electron, no Node.js, no NAPI.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;
use winit::{
    application::ApplicationHandler,
    event::{WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(target_os = "macos")]
use cocoa::base::{id, nil};
#[cfg(target_os = "macos")]
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
#[cfg(target_os = "macos")]
use objc::declare::ClassDecl;
#[cfg(target_os = "macos")]
use objc::runtime::{Class, Object, Sel};
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};

use sujay_audio::engine_core::{AudioEngineCore, DeviceConfigCore, EngineStateUpdate, list_output_devices};
use sujay_ui::{
    attach_raw, detach_raw, set_frame_raw, poll_actions_raw, push_mouse_event_raw,
    set_console_state_raw, set_deck_progress_raw, set_preferences_state_raw,
};

#[cfg(target_os = "macos")]
static PREFS_REQUESTED: AtomicBool = AtomicBool::new(false);

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
    audio_devices: &[sujay_ui::ui_state::AudioDeviceInfo],
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
    if prefs.recording_naming_strategy != "timestamp" && prefs.recording_naming_strategy != "sequential" {
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
    state: sujay_ui::ui_state::PreferencesState,
    audio_devices: &[sujay_ui::ui_state::AudioDeviceInfo],
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
    if format == "ogg" { "ogg" } else { "wav" }
}

fn prepare_recording_path(prefs: &AppPreferences) -> Result<PathBuf, String> {
    let rec_dir = PathBuf::from(&prefs.recording_directory);
    if !rec_dir.is_absolute() {
        return Err("recording directory must be an absolute path".to_owned());
    }

    if !rec_dir.exists() {
        if !prefs.recording_auto_create_directory {
            return Err(format!("recording directory not found: {}", rec_dir.display()));
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
    let parent = path.parent().ok_or_else(|| "Invalid settings path".to_owned())?;
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
        main_channels: Some(prefs.main_channels.iter().map(|v| v.unwrap_or(-1)).collect()),
        cue_channels: Some(prefs.cue_channels.iter().map(|v| v.unwrap_or(-1)).collect()),
    }
}

fn ui_preferences_state(
    prefs: &AppPreferences,
    audio_devices: &[sujay_ui::ui_state::AudioDeviceInfo],
) -> sujay_ui::ui_state::PreferencesState {
    sujay_ui::ui_state::PreferencesState {
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

fn available_audio_devices() -> Vec<sujay_ui::ui_state::AudioDeviceInfo> {
    list_output_devices()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, max_output_channels)| sujay_ui::ui_state::AudioDeviceInfo {
            name,
            max_output_channels,
        })
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

        unsafe {
            decl.add_method(
                sel!(openPreferences:),
                open_preferences as extern "C" fn(&Object, Sel, id),
            );
        }
        decl.register();
    });
    Class::get("SujayMenuHandler").expect("SujayMenuHandler class")
}

#[cfg(target_os = "macos")]
fn nsstring_to_string(s: id) -> String {
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

// ── Settings dialog helpers ───────────────────────────────────────────────────

/// Popup references passed to ObjC action callbacks via thread-local.
#[cfg(target_os = "macos")]
struct ChannelPopupContext {
    device_popup: id,
    channel_popups: [id; 4], // main_l, main_r, cue_l, cue_r
    /// max_output_channels per device popup item: index 0 = System Default (i32::MAX)
    device_max_channels: Vec<i32>,
}

#[cfg(target_os = "macos")]
std::thread_local! {
    static CHANNEL_CTX: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

#[cfg(target_os = "macos")]
fn settings_dialog_class() -> *const objc::runtime::Class {
    use std::sync::Once;
    static INIT: Once = Once::new();
    static mut CLASS: *const objc::runtime::Class = std::ptr::null();
    INIT.call_once(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("SujaySettingsDialogHandler", superclass).unwrap();
        decl.add_method(
            sel!(saveSettings:),
            settings_dialog_save as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(cancelSettings:),
            settings_dialog_cancel as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(channelChanged:),
            settings_channel_changed as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(deviceChanged:),
            settings_device_changed as extern "C" fn(&Object, Sel, id),
        );
        CLASS = decl.register();
    });
    unsafe { CLASS }
}

#[cfg(target_os = "macos")]
extern "C" fn settings_dialog_save(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModalWithCode: 1000i64];
    }
}

#[cfg(target_os = "macos")]
extern "C" fn settings_dialog_cancel(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModalWithCode: 1001i64];
    }
}

/// Edit-time uniqueness: when a channel popup changes, clear other popups that
/// happen to have the same channel selected (first-wins, like the old Electron UI).
#[cfg(target_os = "macos")]
extern "C" fn settings_channel_changed(_this: &Object, _cmd: Sel, sender: id) {
    CHANNEL_CTX.with(|cell| {
        let ptr = cell.get();
        if ptr == 0 { return; }
        let ctx = unsafe { &*(ptr as *const ChannelPopupContext) };
        let changed_idx: i64 = unsafe { msg_send![sender, indexOfSelectedItem] };
        if changed_idx <= 0 { return; } // "-" selected – nothing to de-dup
        let changed_val = (changed_idx - 1) as i32;
        for popup in &ctx.channel_popups {
            if *popup != sender {
                let idx: i64 = unsafe { msg_send![*popup, indexOfSelectedItem] };
                if idx > 0 && (idx - 1) as i32 == changed_val {
                    unsafe { let _: () = msg_send![*popup, selectItemAtIndex: 0i64]; }
                }
            }
        }
    });
}

/// Edit-time clamping: when the device popup changes, rebuild channel popup items
/// to only show channels the new device supports, then clamp any out-of-range selection.
#[cfg(target_os = "macos")]
extern "C" fn settings_device_changed(_this: &Object, _cmd: Sel, _sender: id) {
    CHANNEL_CTX.with(|cell| {
        let ptr = cell.get();
        if ptr == 0 { return; }
        let ctx = unsafe { &*(ptr as *const ChannelPopupContext) };
        let dev_idx: i64 = unsafe { msg_send![ctx.device_popup, indexOfSelectedItem] };
        let max_ch = ctx.device_max_channels
            .get(dev_idx as usize)
            .copied()
            .unwrap_or(i32::MAX);
        for popup in &ctx.channel_popups {
            let prev_idx: i64 = unsafe { msg_send![*popup, indexOfSelectedItem] };
            // Rebuild items so the list reflects the new device's channel count.
            unsafe {
                let _: () = msg_send![*popup, removeAllItems];
                let _: () = msg_send![*popup, addItemWithTitle: NSString::alloc(nil).init_str("-")];
                for ch in 0..max_ch {
                    let title = format!("{}", ch + 1);
                    let _: () = msg_send![*popup, addItemWithTitle:
                        NSString::alloc(nil).init_str(&title)];
                }
                let count: i64 = msg_send![*popup, numberOfItems];
                let clamped = if prev_idx < count { prev_idx } else { 0 };
                let _: () = msg_send![*popup, selectItemAtIndex: clamped];
            }
        }
    });
}

#[cfg(target_os = "macos")]
unsafe fn show_native_preferences_dialog(
    current: &sujay_ui::ui_state::PreferencesState,
) -> Option<sujay_ui::ui_state::PreferencesState> {
    let _pool = NSAutoreleasePool::new(nil);

    // ── NSPanel (no icon area, full layout control) ───────────────────────────
    let style_mask: u64 = 1 | 2; // NSTitledWindowMask | NSClosableWindowMask
    let panel: id = msg_send![class!(NSPanel), alloc];
    let panel: id = msg_send![panel,
        initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 440.0))
        styleMask: style_mask
        backing: 2u64
        defer: false];
    let _: () = msg_send![panel, setTitle: NSString::alloc(nil).init_str("Settings")];
    let _: () = msg_send![panel, center];
    let content_view: id = msg_send![panel, contentView];

    // ── Tab view (full width, above button bar) ───────────────────────────────
    let tab_view: id = msg_send![class!(NSTabView), alloc];
    let tab_view: id = msg_send![tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 52.0), NSSize::new(480.0, 388.0))];
    let _: () = msg_send![content_view, addSubview: tab_view];

    // ── Buttons ───────────────────────────────────────────────────────────────
    let btn_handler: id = msg_send![settings_dialog_class(), new];

    let save_btn: id = msg_send![class!(NSButton), alloc];
    let save_btn: id = msg_send![save_btn, initWithFrame:
        NSRect::new(NSPoint::new(384.0, 12.0), NSSize::new(80.0, 28.0))];
    let _: () = msg_send![save_btn, setTitle: NSString::alloc(nil).init_str("Save")];
    let _: () = msg_send![save_btn, setBezelStyle: 1u64];
    let _: () = msg_send![save_btn, setTarget: btn_handler];
    let _: () = msg_send![save_btn, setAction: sel!(saveSettings:)];
    let _: () = msg_send![save_btn, setKeyEquivalent: NSString::alloc(nil).init_str("\r")];
    let _: () = msg_send![content_view, addSubview: save_btn];

    let cancel_btn: id = msg_send![class!(NSButton), alloc];
    let cancel_btn: id = msg_send![cancel_btn, initWithFrame:
        NSRect::new(NSPoint::new(296.0, 12.0), NSSize::new(84.0, 28.0))];
    let _: () = msg_send![cancel_btn, setTitle: NSString::alloc(nil).init_str("Cancel")];
    let _: () = msg_send![cancel_btn, setBezelStyle: 1u64];
    let _: () = msg_send![cancel_btn, setTarget: btn_handler];
    let _: () = msg_send![cancel_btn, setAction: sel!(cancelSettings:)];
    let _: () = msg_send![cancel_btn, setKeyEquivalent: NSString::alloc(nil).init_str("\x1b")];
    let _: () = msg_send![content_view, addSubview: cancel_btn];

    // ── Helper: static non-editable label ────────────────────────────────────
    let mk_label = |x: f64, y: f64, w: f64, text: &str| -> id {
        let f: id = unsafe { msg_send![class!(NSTextField), alloc] };
        let f: id = unsafe { msg_send![f, initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(w, 20.0))] };
        let _: () = unsafe { msg_send![f, setStringValue: NSString::alloc(nil).init_str(text)] };
        let _: () = unsafe { msg_send![f, setEditable: false] };
        let _: () = unsafe { msg_send![f, setBordered: false] };
        let _: () = unsafe { msg_send![f, setDrawsBackground: false] };
        f
    };

    // ── Tab: Audio ────────────────────────────────────────────────────────────
    // Tab content area ≈ 360 px tall (388 tabview − ~28 tab bar).
    // 16 pt padding on all sides; items flow top-to-bottom.
    let audio_tab_item: id = msg_send![class!(NSTabViewItem), alloc];
    let audio_tab_item: id = msg_send![audio_tab_item, initWithIdentifier: nil];
    let _: () = msg_send![audio_tab_item, setLabel: NSString::alloc(nil).init_str("Audio")];
    let audio_tab_view: id = msg_send![class!(NSView), alloc];
    let audio_tab_view: id = msg_send![audio_tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0))];

    let dev_label = mk_label(16.0, 324.0, 448.0, "Audio Device");

    let device_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let device_popup: id = msg_send![device_popup, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 294.0), NSSize::new(448.0, 26.0)) pullsDown: false];
    let _: () = msg_send![device_popup, addItemWithTitle: NSString::alloc(nil).init_str("System Default")];
    for dev in &current.audio_devices {
        let title = format!("{} ({} ch)", dev.name, dev.max_output_channels);
        let _: () = msg_send![device_popup, addItemWithTitle: NSString::alloc(nil).init_str(&title)];
    }
    if let Some(ref id) = current.audio_device_id {
        if let Some((idx, _)) = current.audio_devices.iter().enumerate().find(|(_, d)| &d.name == id) {
            let _: () = msg_send![device_popup, selectItemAtIndex: (idx + 1) as i64];
        }
    }

    let selected_max_channels = current
        .audio_device_id
        .as_ref()
        .and_then(|id| current.audio_devices.iter().find(|d| &d.name == id))
        .map(|d| d.max_output_channels as i32)
        .unwrap_or(2)
        .max(2);

    let routing_label = mk_label(16.0, 256.0, 448.0, "Output Routing");

    let mk_channel_popup = |x: f64, y: f64, selected: Option<i32>| {
        let popup: id = unsafe { msg_send![class!(NSPopUpButton), alloc] };
        let popup: id = unsafe { msg_send![popup, initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(110.0, 26.0)) pullsDown: false] };
        let _: () = unsafe { msg_send![popup, addItemWithTitle: NSString::alloc(nil).init_str("-")] };
        for ch in 0..selected_max_channels {
            let title = format!("{}", ch + 1);
            let _: () = unsafe { msg_send![popup, addItemWithTitle: NSString::alloc(nil).init_str(&title)] };
        }
        let idx = selected.map(|v| (v + 1) as i64).unwrap_or(0).max(0);
        let _: () = unsafe { msg_send![popup, selectItemAtIndex: idx] };
        popup
    };

    // Two-column channel routing (L / R), 16 pt left padding, 248 px column gap
    let main_l_label = mk_label(16.0, 228.0, 60.0, "Main L");
    let main_l_popup = mk_channel_popup(16.0, 198.0, current.main_channels[0]);
    let main_r_label = mk_label(248.0, 228.0, 60.0, "Main R");
    let main_r_popup = mk_channel_popup(248.0, 198.0, current.main_channels[1]);
    let cue_l_label  = mk_label(16.0, 160.0, 60.0, "Cue L");
    let cue_l_popup  = mk_channel_popup(16.0, 130.0, current.cue_channels[0]);
    let cue_r_label  = mk_label(248.0, 160.0, 60.0, "Cue R");
    let cue_r_popup  = mk_channel_popup(248.0, 130.0, current.cue_channels[1]);

    // ── Connect edit-time uniqueness + clamping callbacks ─────────────────────
    let ctx = Box::new(ChannelPopupContext {
        device_popup,
        channel_popups: [main_l_popup, main_r_popup, cue_l_popup, cue_r_popup],
        device_max_channels: {
            let mut v = vec![i32::MAX]; // index 0 = System Default
            for dev in &current.audio_devices {
                v.push(dev.max_output_channels as i32);
            }
            v
        },
    });
    let ctx_ptr = Box::into_raw(ctx) as usize;
    CHANNEL_CTX.with(|cell| cell.set(ctx_ptr));

    // Device popup → clamp channel items on change
    let _: () = msg_send![device_popup, setTarget: btn_handler];
    let _: () = msg_send![device_popup, setAction: sel!(deviceChanged:)];
    // Channel popups → enforce uniqueness on change
    for popup in &[main_l_popup, main_r_popup, cue_l_popup, cue_r_popup] {
        let _: () = msg_send![*popup, setTarget: btn_handler];
        let _: () = msg_send![*popup, setAction: sel!(channelChanged:)];
    }

    for sv in &[dev_label, device_popup, routing_label,
                main_l_label, main_l_popup, main_r_label, main_r_popup,
                cue_l_label,  cue_l_popup,  cue_r_label,  cue_r_popup] {
        let _: () = msg_send![audio_tab_view, addSubview: *sv];
    }
    let _: () = msg_send![audio_tab_item, setView: audio_tab_view];
    let _: () = msg_send![tab_view, addTabViewItem: audio_tab_item];

    // ── Tab: Recording ────────────────────────────────────────────────────────
    let recording_tab_item: id = msg_send![class!(NSTabViewItem), alloc];
    let recording_tab_item: id = msg_send![recording_tab_item, initWithIdentifier: nil];
    let _: () = msg_send![recording_tab_item, setLabel: NSString::alloc(nil).init_str("Recording")];
    let recording_tab_view: id = msg_send![class!(NSView), alloc];
    let recording_tab_view: id = msg_send![recording_tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0))];

    let rec_dir_label = mk_label(16.0, 324.0, 448.0, "Recording Directory");

    let rec_dir_field: id = msg_send![class!(NSTextField), alloc];
    let rec_dir_field: id = msg_send![rec_dir_field, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 294.0), NSSize::new(448.0, 26.0))];
    let _: () = msg_send![rec_dir_field, setStringValue: NSString::alloc(nil).init_str(&current.recording_directory)];

    let rec_auto_cb: id = msg_send![class!(NSButton), alloc];
    let rec_auto_cb: id = msg_send![rec_auto_cb, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 256.0), NSSize::new(380.0, 20.0))];
    let _: () = msg_send![rec_auto_cb, setButtonType: 3u64];
    let _: () = msg_send![rec_auto_cb, setTitle: NSString::alloc(nil).init_str("Auto-create recording directory")];
    let _: () = msg_send![rec_auto_cb, setState: if current.recording_auto_create_directory { 1i64 } else { 0i64 }];

    let rec_name_label = mk_label(16.0, 224.0, 160.0, "Naming");
    let rec_name_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let rec_name_popup: id = msg_send![rec_name_popup, initWithFrame:
        NSRect::new(NSPoint::new(180.0, 220.0), NSSize::new(284.0, 26.0)) pullsDown: false];
    let _: () = msg_send![rec_name_popup, addItemWithTitle: NSString::alloc(nil).init_str("timestamp")];
    let _: () = msg_send![rec_name_popup, addItemWithTitle: NSString::alloc(nil).init_str("sequential")];
    let rec_name_idx = if current.recording_naming_strategy == "sequential" { 1 } else { 0 };
    let _: () = msg_send![rec_name_popup, selectItemAtIndex: rec_name_idx];

    let rec_fmt_label = mk_label(16.0, 184.0, 160.0, "Format");
    let fmt_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let fmt_popup: id = msg_send![fmt_popup, initWithFrame:
        NSRect::new(NSPoint::new(180.0, 180.0), NSSize::new(180.0, 26.0)) pullsDown: false];
    let _: () = msg_send![fmt_popup, addItemWithTitle: NSString::alloc(nil).init_str("wav")];
    let _: () = msg_send![fmt_popup, addItemWithTitle: NSString::alloc(nil).init_str("ogg")];
    let fmt_idx = if current.recording_format == "ogg" { 1 } else { 0 };
    let _: () = msg_send![fmt_popup, selectItemAtIndex: fmt_idx];

    for sv in &[rec_dir_label, rec_dir_field, rec_auto_cb,
                rec_name_label, rec_name_popup, rec_fmt_label, fmt_popup] {
        let _: () = msg_send![recording_tab_view, addSubview: *sv];
    }
    let _: () = msg_send![recording_tab_item, setView: recording_tab_view];
    let _: () = msg_send![tab_view, addTabViewItem: recording_tab_item];

    // ── Tab: OSC ──────────────────────────────────────────────────────────────
    let osc_tab_item: id = msg_send![class!(NSTabViewItem), alloc];
    let osc_tab_item: id = msg_send![osc_tab_item, initWithIdentifier: nil];
    let _: () = msg_send![osc_tab_item, setLabel: NSString::alloc(nil).init_str("OSC")];
    let osc_tab_view: id = msg_send![class!(NSView), alloc];
    let osc_tab_view: id = msg_send![osc_tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0))];

    let osc_enabled_cb: id = msg_send![class!(NSButton), alloc];
    let osc_enabled_cb: id = msg_send![osc_enabled_cb, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 324.0), NSSize::new(380.0, 20.0))];
    let _: () = msg_send![osc_enabled_cb, setButtonType: 3u64];
    let _: () = msg_send![osc_enabled_cb, setTitle: NSString::alloc(nil).init_str("Enable OSC")];
    let _: () = msg_send![osc_enabled_cb, setState: if current.osc_enabled { 1i64 } else { 0i64 }];

    let osc_host_label = mk_label(16.0, 286.0, 100.0, "Host");
    let osc_host_field: id = msg_send![class!(NSTextField), alloc];
    let osc_host_field: id = msg_send![osc_host_field, initWithFrame:
        NSRect::new(NSPoint::new(120.0, 282.0), NSSize::new(228.0, 26.0))];
    let _: () = msg_send![osc_host_field, setStringValue: NSString::alloc(nil).init_str(&current.osc_host)];

    let osc_port_label = mk_label(356.0, 286.0, 44.0, "Port");
    let osc_port_field: id = msg_send![class!(NSTextField), alloc];
    let osc_port_field: id = msg_send![osc_port_field, initWithFrame:
        NSRect::new(NSPoint::new(404.0, 282.0), NSSize::new(60.0, 26.0))];
    let _: () = msg_send![osc_port_field, setStringValue: NSString::alloc(nil).init_str(&current.osc_port.to_string())];

    for sv in &[osc_enabled_cb, osc_host_label, osc_host_field, osc_port_label, osc_port_field] {
        let _: () = msg_send![osc_tab_view, addSubview: *sv];
    }
    let _: () = msg_send![osc_tab_item, setView: osc_tab_view];
    let _: () = msg_send![tab_view, addTabViewItem: osc_tab_item];

    // ── Run modal ─────────────────────────────────────────────────────────────
    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let code: i64 = msg_send![app, runModalForWindow: panel];
    let _: () = msg_send![panel, orderOut: nil];
    // Free context and clear thread-local regardless of result.
    CHANNEL_CTX.with(|cell| {
        let ptr = cell.get();
        if ptr != 0 {
            drop(Box::from_raw(ptr as *mut ChannelPopupContext));
            cell.set(0);
        }
    });
    if code != 1000 {
        return None;
    }

    let selected_idx: i64 = msg_send![device_popup, indexOfSelectedItem];
    let selected_device = if selected_idx <= 0 {
        None
    } else {
        current.audio_devices.get((selected_idx - 1) as usize).map(|d| d.name.clone())
    };

    // Max channels for the newly-selected device (used for clamping).
    let new_max_channels = selected_device
        .as_ref()
        .and_then(|name| current.audio_devices.iter().find(|d| &d.name == name))
        .map(|d| d.max_output_channels as i32)
        .unwrap_or(i32::MAX);

    let read_channel = |popup: id| -> Option<i32> {
        let idx: i64 = unsafe { msg_send![popup, indexOfSelectedItem] };
        if idx <= 0 { None } else { Some((idx - 1) as i32) }
    };

    // Read all 4 slots, then apply clamping + uniqueness (first-wins order:
    // Main L → Main R → Cue L → Cue R, same as the original Electron UI).
    let raw: [Option<i32>; 4] = [
        read_channel(main_l_popup),
        read_channel(main_r_popup),
        read_channel(cue_l_popup),
        read_channel(cue_r_popup),
    ];
    let mut seen = std::collections::HashSet::<i32>::new();
    let resolved: Vec<Option<i32>> = raw.iter().map(|ch| match ch {
        Some(v) if *v < new_max_channels && seen.insert(*v) => Some(*v),
        _ => None,
    }).collect();
    let main_channels = [resolved[0], resolved[1]];
    let cue_channels  = [resolved[2], resolved[3]];

    let dir_value: id = msg_send![rec_dir_field, stringValue];
    let rec_auto_state: i64 = msg_send![rec_auto_cb, state];
    let rec_name_idx: i64 = msg_send![rec_name_popup, indexOfSelectedItem];
    let fmt_selected: i64 = msg_send![fmt_popup, indexOfSelectedItem];
    let osc_enabled_state: i64 = msg_send![osc_enabled_cb, state];
    let osc_host_value: id = msg_send![osc_host_field, stringValue];
    let osc_port_value: id = msg_send![osc_port_field, stringValue];

    let mut next = current.clone();
    next.audio_device_id = selected_device;
    next.main_channels = main_channels;
    next.cue_channels = cue_channels;
    next.recording_directory = nsstring_to_string(dir_value);
    next.recording_auto_create_directory = rec_auto_state != 0;
    next.recording_naming_strategy = if rec_name_idx == 1 {
        "sequential".to_owned()
    } else {
        "timestamp".to_owned()
    };
    next.recording_format = if fmt_selected == 1 { "ogg".to_owned() } else { "wav".to_owned() };
    next.osc_enabled = osc_enabled_state != 0;
    next.osc_host = nsstring_to_string(osc_host_value);
    next.osc_port = nsstring_to_string(osc_port_value)
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .unwrap_or(current.osc_port);
    Some(next)
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
    /// System info sampler (CPU / memory).
    sys: sysinfo::System,
    /// Last whole-second timestamp used for titlebar system-info refresh.
    last_titlebar_second: Option<u64>,
    /// Cached titlebar system fields that only need 1 Hz refresh.
    cached_titlebar: sujay_ui::ui_state::TitlebarState,
    /// Last full console snapshot submitted to the native renderer.
    last_console_visual: Option<sujay_ui::ui_state::ConsoleVisualState>,
    /// Last deck progress tuples submitted to the renderer: (pos, total, sr).
    last_deck_progress: [Option<(f32, f32, f32)>; 2],
    /// Timestamp when the current recording session started (None = not recording).
    rec_started_at: Option<Instant>,
    settings_path: PathBuf,
    preferences: AppPreferences,
    audio_devices: Vec<sujay_ui::ui_state::AudioDeviceInfo>,
}

impl SujayApp {
    fn new() -> Self {
        let (decode_tx, decode_rx) = mpsc::channel();
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
            sys,
            last_titlebar_second: None,
            cached_titlebar: sujay_ui::ui_state::TitlebarState::default(),
            last_console_visual: None,
            last_deck_progress: [None, None],
            rec_started_at: None,
            settings_path,
            preferences,
            audio_devices: vec![],
        }
    }

    fn dispatch_action(&mut self, action: sujay_ui::UiAction) {
        use sujay_ui::UiAction;

        let Some(engine) = self.engine.as_ref().cloned() else { return; };

        match action {
            UiAction::Play(deck) => { let _ = engine.play(deck as u32); }
            UiAction::Stop(deck) => { let _ = engine.stop(deck as u32); }
            UiAction::SetCrossfader(v) => { let _ = engine.set_crossfader_position(v as f64); }
            UiAction::SetMasterTempo(v) => { let _ = engine.set_master_tempo(v as f64); }
            UiAction::SetDeckGain(deck, v) => { let _ = engine.set_deck_gain(deck as u32, v as f64); }
            UiAction::SetCue(deck, enabled) => { let _ = engine.set_deck_cue_enabled(deck as u32, enabled); }
            UiAction::SetEq(deck, band, kill) => { let _ = engine.set_eq_cut(deck as u32, band, kill); }
            UiAction::Seek(deck, pos) => { let _ = engine.seek(deck as u32, pos as f64); }
            UiAction::ToggleLoop(deck, beats) => {
                if beats <= 0.0 {
                    let _ = engine.clear_loop(deck as u32);
                } else if let Some((beat_grid, current_pos)) = sujay_ui::get_deck_beat_info_raw(deck as u32) {
                    let start_beat_idx = beat_grid.partition_point(|&b| b <= current_pos).saturating_sub(1);
                    let start_frames = beat_grid.get(start_beat_idx).copied().unwrap_or(current_pos);
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
            UiAction::LoadFile(deck, path) => {
                spawn_decode(deck, PathBuf::from(path), self.decode_tx.clone());
            }
            UiAction::SetMicEnabled(enabled) => {
                let _ = engine.set_mic_enabled(enabled);
            }
            UiAction::StartRecording => {
                match prepare_recording_path(&self.preferences) {
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
                }
            }
            UiAction::StopRecording => {
                let _ = engine.stop_recording();
            }
            UiAction::OpenPreferences => {
                self.open_native_preferences_dialog();
            }
            UiAction::SavePreferences(state) => {
                apply_preferences_state(&mut self.preferences, state, &self.audio_devices);

                if let Err(err) = save_preferences(&self.settings_path, &self.preferences) {
                    tracing::warn!("failed to save preferences: {}", err);
                }
                if let Err(err) = engine.configure_device(device_config_from_preferences(&self.preferences)) {
                    tracing::warn!("failed to apply audio preferences: {}", err);
                }
                set_preferences_state_raw(ui_preferences_state(&self.preferences, &self.audio_devices));
            }
        }
    }

    fn open_native_preferences_dialog(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            let current = ui_preferences_state(&self.preferences, &self.audio_devices);
            if let Some(next) = show_native_preferences_dialog(&current) {
                apply_preferences_state(&mut self.preferences, next, &self.audio_devices);

                if let Err(err) = save_preferences(&self.settings_path, &self.preferences) {
                    tracing::warn!("failed to save preferences: {}", err);
                }
                if let Some(engine) = self.engine.as_ref() {
                    if let Err(err) = engine.configure_device(device_config_from_preferences(&self.preferences)) {
                        tracing::warn!("failed to apply audio preferences: {}", err);
                    }
                }
                set_preferences_state_raw(ui_preferences_state(&self.preferences, &self.audio_devices));
            }
        }
    }
}

impl ApplicationHandler for SujayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }

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
        let logical = window.inner_size().to_logical::<f64>(scale);

        // ── Attach native UI renderer ────────────────────────────────────────
        #[cfg(target_os = "macos")]
        {
            unsafe { install_macos_app_menu(); }
            let ns_view = match window.window_handle().unwrap().as_raw() {
                RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut std::ffi::c_void,
                _ => panic!("unexpected window handle type"),
            };
            attach_raw(ns_view, 0.0, 0.0, logical.width, logical.height);
        }

        // ── Start audio engine ───────────────────────────────────────────────
        let last_state = Arc::clone(&self.last_state);
        let engine = Arc::new(AudioEngineCore::new(
            Some(44100),
            Arc::new(move |state: EngineStateUpdate| {
                if let Ok(mut guard) = last_state.lock() { *guard = Some(state); }
            }),
        )
        .expect("Failed to initialise audio engine"));

        self.audio_devices = available_audio_devices();
        normalize_preferences(&mut self.preferences, &self.audio_devices);
        if let Err(err) = engine.configure_device(device_config_from_preferences(&self.preferences)) {
            tracing::warn!("failed to configure initial audio device: {}", err);
        }
        set_preferences_state_raw(ui_preferences_state(&self.preferences, &self.audio_devices));

        self.window = Some(window);
        self.engine = Some(engine);
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

                // Drag the window when the user presses in the middle titlebar area
                // (between the traffic lights on the left and the info controls on the right).
                // drag_window() is non-blocking on a non-drag click and uses the current
                // NSEvent, so it must be called here (inside the window_event handler).
                if state == winit::event::ElementState::Pressed && self.cursor_pos.1 < 38.0 {
                    let win_w = self.window.as_ref()
                        .map(|w| w.inner_size().to_logical::<f32>(w.scale_factor()).width)
                        .unwrap_or(1100.0);
                    // x > 80: past traffic lights; x < w-360: before right-side controls
                    let in_drag_area = self.cursor_pos.0 > 80.0 && self.cursor_pos.0 < win_w - 360.0;
                    if in_drag_area {
                        if let Some(ref win) = self.window {
                            let _ = win.drag_window();
                        }
                    }
                }
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut needs_redraw = false;

        #[cfg(target_os = "macos")]
        if PREFS_REQUESTED.swap(false, Ordering::Relaxed) {
            self.open_native_preferences_dialog();
            needs_redraw = true;
        }

        // Drain UI actions and dispatch to engine
        if self.engine.is_some() {
            let mut handled_any_action = false;
            for action in poll_actions_raw() {
                self.dispatch_action(action);
                handled_any_action = true;
            }
            needs_redraw |= handled_any_action;
        }

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
                            unsafe { libc::localtime_r(&ts, &mut tm); }
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
                    self.cached_titlebar.mem_mb = self.sys.process(pid)
                        .map(|p| p.memory() / 1024 / 1024)
                        .unwrap_or(0);
                }

                let rec_elapsed_secs = self.rec_started_at
                    .map(|t| t.elapsed().as_secs() as u32)
                    .unwrap_or(0);

                let mut cv = engine_state_to_console_visual(&state);
                cv.titlebar = sujay_ui::ui_state::TitlebarState {
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
                    let next = (pos as f32, state.deck_a_total_frames.unwrap_or(0.0) as f32, sr);
                    if self.last_deck_progress[0] != Some(next) {
                        self.last_deck_progress[0] = Some(next);
                        set_deck_progress_raw(1, next.0, next.1, next.2);
                        needs_redraw = true;
                    }
                }
                if let Some(pos) = state.deck_b_position {
                    let next = (pos as f32, state.deck_b_total_frames.unwrap_or(0.0) as f32, sr);
                    if self.last_deck_progress[1] != Some(next) {
                        self.last_deck_progress[1] = Some(next);
                        set_deck_progress_raw(2, next.0, next.1, next.2);
                        needs_redraw = true;
                    }
                }
            }
        }

        // Drain completed background decodes and load into engine on main thread
        if let Some(ref engine) = self.engine {
            while let Ok(ready) = self.decode_rx.try_recv() {
                needs_redraw = true;
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

        // Wake at the audio-state cadence instead of spinning the main thread.
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33)));

        if needs_redraw {
            if let Some(ref w) = self.window {
                w.request_redraw();
            }
        }
    }
}

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
        titlebar: Default::default(),   // overwritten in about_to_wait with live data
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
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = SujayApp::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
