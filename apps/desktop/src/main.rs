#![allow(unexpected_cfgs)]

//! Sujay — Rust-native host binary.
//!
//! Creates a winit window, attaches the wgpu/egui UI renderer and drives
//! [`sujay_core::Core`] from the event loop. Everything that is not windowing,
//! menus or the native settings dialog lives in `crates/core`.

use std::path::PathBuf;
use std::sync::Arc;
// Used by the macOS menu's PREFS_REQUESTED flag; the Windows menu module has its own.
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};
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

use sujay_core::{Core, Tick};
#[cfg(not(target_os = "macos"))]
use sujay_decks::push_mouse_event_raw;
use sujay_decks::{
    attach_raw, detach_raw, poll_actions_raw, set_console_state_raw, set_deck_markers_raw,
    set_deck_progress_raw, set_frame_raw, set_library_state_raw, set_preferences_state_raw,
    set_waveform_colors_raw, set_waveform_raw, UiAction,
};

#[cfg(target_os = "macos")]
static PREFS_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static LIBRARY_MENU_SELECTION: AtomicU8 = AtomicU8::new(0);

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
    core: Core,
    /// Last known cursor position in logical points (top-left origin).
    cursor_pos: (f32, f32),
    /// Deck selected during hover phase (1=A, 2=B).
    hovered_deck: u8,
}

impl SujayApp {
    fn new() -> Self {
        Self {
            window: None,
            core: Core::new(),
            cursor_pos: (0.0, 0.0),
            hovered_deck: 1,
        }
    }

    fn dispatch_action(&mut self, action: UiAction) {
        match action {
            UiAction::LoadFile(deck, path) => {
                #[cfg(target_os = "macos")]
                unsafe {
                    if let Some(window) = self.window.as_ref() {
                        restore_main_view_first_responder(window);
                    }
                }
                self.core.load_file(deck, PathBuf::from(path));
            }
            UiAction::OpenPreferences => {
                self.open_native_preferences_dialog();
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
            }
            UiAction::Play(deck) => self.core.play(deck),
            UiAction::Stop(deck) => self.core.stop(deck),
            UiAction::SetCrossfader(v) => self.core.set_crossfader(v),
            UiAction::SetMasterTempo(v) => self.core.set_master_tempo(v),
            UiAction::SetDeckGain(deck, v) => self.core.set_deck_gain(deck, v),
            UiAction::SetCue(deck, enabled) => self.core.set_cue(deck, enabled),
            UiAction::SetEq(deck, band, kill) => self.core.set_eq(deck, band, kill),
            UiAction::Seek(deck, pos) => self.core.seek(deck, pos),
            UiAction::RecallCue(deck, position, loop_end) => {
                self.core.recall_cue(deck, position, loop_end)
            }
            UiAction::ToggleLoop(deck, beats) => self.core.toggle_loop(deck, beats),
            UiAction::SetMicEnabled(enabled) => self.core.set_mic_enabled(enabled),
            UiAction::StartRecording => self.core.start_recording(),
            UiAction::StopRecording => self.core.stop_recording(),
            UiAction::SavePreferences(state) => self.core.apply_preferences(state),
        }
    }

    fn open_native_preferences_dialog(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            let current = self.core.preferences_state();
            if let Some(next) = mac_settings::show_native_preferences_dialog(&current) {
                self.core.apply_preferences(next);
            }
        }
    }

    /// Push whatever `tick` changed into the egui renderer.
    fn push_changes(&self, tick: &Tick) {
        if tick.preferences {
            set_preferences_state_raw(self.core.preferences_state());
        }
        if tick.library {
            set_library_state_raw(self.core.library_state().clone());
        }
        if tick.console {
            set_console_state_raw(self.core.console_state().clone());
        }
        for (idx, deck) in [1u8, 2u8].into_iter().enumerate() {
            let buffers = self.core.deck(deck);
            if tick.waveform[idx] {
                set_waveform_raw(deck as u32, buffers.waveform.clone());
                set_waveform_colors_raw(deck as u32, buffers.waveform_colors.clone());
            }
            if tick.markers[idx] {
                set_deck_markers_raw(
                    deck as u32,
                    buffers.beats.clone(),
                    buffers.intro,
                    buffers.outro,
                );
            }
            if tick.progress[idx] {
                if let Some((position, total, sample_rate)) = buffers.progress {
                    set_deck_progress_raw(deck as u32, position, total, sample_rate);
                }
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

        self.core
            .start()
            .expect("Failed to initialise audio engine");

        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("window close requested");
                self.core.shutdown();
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
                self.core.load_file(self.hovered_deck, path);
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
                self.core.refresh_audio_devices();
                let current = self.core.preferences_state();
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
                self.core.apply_preferences(next);
                needs_redraw = true;
            }
        }

        // Drain UI actions every frame. Some actions (e.g. library load) are
        // valid before/without an initialized audio engine.
        for action in poll_actions_raw() {
            self.dispatch_action(action);
            needs_redraw = true;
        }

        let tick = self.core.tick();
        self.push_changes(&tick);
        needs_redraw |= tick.any();

        if tick.retry_soon {
            // A decoded track is waiting for the engine; come back almost at once.
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(1),
            ));
            return;
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

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Sujay starting (Rust-native host)");

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = SujayApp::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
