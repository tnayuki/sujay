#![allow(unexpected_cfgs)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Arc;
#[cfg(not(target_os = "macos"))]
use sujay_decks::push_mouse_event_raw;
use sujay_decks::ui_state::{
    ConsoleVisualState, DeckConsoleVisualState, LibraryTrackItem, LibraryVisualState,
};
use sujay_decks::{
    attach_raw, detach_raw, poll_actions_raw, set_console_state_raw, set_frame_raw,
    set_library_state_raw,
};
#[cfg(not(target_os = "macos"))]
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

struct InputProbeApp {
    window: Option<Arc<Window>>,
    cursor_pos: (f32, f32),
}

impl InputProbeApp {
    fn new() -> Self {
        Self {
            window: None,
            cursor_pos: (0.0, 0.0),
        }
    }

    fn seed_ui_state(&self) {
        let mut state = ConsoleVisualState::default();
        state.deck_a = DeckConsoleVisualState {
            title: "Deck A".to_owned(),
            bpm_text: "124.0".to_owned(),
            bpm: 124.0,
            ..DeckConsoleVisualState::default()
        };
        state.deck_b = DeckConsoleVisualState {
            title: "Deck B".to_owned(),
            bpm_text: "126.0".to_owned(),
            bpm: 126.0,
            ..DeckConsoleVisualState::default()
        };
        set_console_state_raw(state);

        let tracks = (0..40)
            .map(|i| LibraryTrackItem {
                id: format!("probe-track-{i}"),
                title: format!("Probe Track {i:02}"),
                artist: format!("Artist {}", i % 6),
                album: format!("Album {}", i % 4),
                bpm: Some(120.0 + (i % 8) as f32),
                duration_seconds: Some(140.0 + (i * 3) as f32),
                rating: Some((i % 5) as i32 + 1),
                tags: Some("probe,test,input".to_owned()),
                release_date: Some("2026-07-18".to_owned()),
                file_path: format!("/tmp/probe_track_{i:02}.mp3"),
            })
            .collect::<Vec<_>>();
        set_library_state_raw(LibraryVisualState {
            source_label: "Input Probe".to_owned(),
            tracks,
            playlists: vec![],
        });
    }
}

impl ApplicationHandler for InputProbeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Sujay Input Probe")
            .with_inner_size(winit::dpi::LogicalSize::new(1120u32, 760u32));

        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        };

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create probe window"),
        );
        let scale = window.scale_factor();
        let logical = window.inner_size().to_logical::<f64>(scale);

        #[cfg(target_os = "macos")]
        {
            let ns_view = match window.window_handle().expect("window handle").as_raw() {
                RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut std::ffi::c_void,
                _ => panic!("unexpected window handle type"),
            };
            attach_raw(ns_view, 0.0, 0.0, logical.width, logical.height);
        }

        #[cfg(target_os = "windows")]
        {
            let hwnd = match window.window_handle().expect("window handle").as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get() as *mut std::ffi::c_void,
                _ => panic!("unexpected window handle type"),
            };
            attach_raw(hwnd, 0.0, 0.0, logical.width, logical.height);
        }

        self.window = Some(window);
        self.seed_ui_state();
        eprintln!("[input-probe] ready: click, right-click, drag in library rows");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
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
                #[cfg(target_os = "macos")]
                let _ = (state, button);
                #[cfg(not(target_os = "macos"))]
                {
                    let kind = match (button, state) {
                        (MouseButton::Left, ElementState::Pressed) => Some(1),
                        (MouseButton::Left, ElementState::Released) => Some(2),
                        (MouseButton::Right, ElementState::Pressed) => Some(3),
                        (MouseButton::Right, ElementState::Released) => Some(4),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        push_mouse_event_raw(kind, self.cursor_pos.0, self.cursor_pos.1);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                #[cfg(target_os = "macos")]
                let _ = delta;
                #[cfg(not(target_os = "macos"))]
                {
                    let wheel_points = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y * 24.0,
                        MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
                    };
                    push_mouse_event_raw(5, 0.0, wheel_points);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        for action in poll_actions_raw() {
            eprintln!("[input-probe] UiAction: {:?}", action);
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = InputProbeApp::new();
    event_loop.run_app(&mut app).expect("run input probe");
}
