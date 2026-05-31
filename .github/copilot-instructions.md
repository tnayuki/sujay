# Sujay - DJ Application

## Project Overview
Sujay is a Rust-native DJ application. It provides a complete DJ experience with dual decks, crossfader, waveform visualization, and local audio file loading. There is no Electron, Node.js, or TypeScript — the entire stack is Rust.

## Current Architecture

### Core Technologies
- **Runtime**: Rust (no Electron, no Node.js)
- **UI**: egui + wgpu, rendered via a Metal/Direct3D/Vulkan surface owned by the native window
- **Audio Engine**: `crates/audio` (cpal + SoundTouch + web-audio-api)
- **Host Binary**: `apps/desktop` — creates the window, drives the event loop, owns the macOS native menu and Settings dialog

### Process Architecture
```
apps/desktop (sujay-app binary)
  ├── winit event loop  ←→  crates/ui (egui renderer, UiAction dispatch)
  └── AudioEngineCore   ←→  crates/audio (real-time processing thread)
```

No worker threads, no IPC, no JS bridge.

## Directory Structure
```
sujay/
├── apps/
│   └── desktop/
│       └── src/main.rs       # Host binary: window, macOS menu, Settings dialog
├── crates/
│   ├── audio/
│   │   └── src/
│   │       ├── engine_core.rs    # AudioEngineCore: deck state, processing thread
│   │       ├── engine_backend.rs # WebAudioBackend: mix/routing/EQ graph
│   │       ├── beat_detector.rs  # BPM detection
│   │       ├── decoder.rs        # MP3/audio file decoding
│   │       └── recorder.rs       # WAV/OGG recording thread
│   └── ui/
│       └── src/
│           ├── renderer.rs       # egui draw loop, UiAction handling
│           ├── ui_state.rs       # Shared state types (ConsoleVisualState, PreferencesState, …)
│           ├── lib.rs            # Public C-ABI surface called from apps/desktop
│           └── waveform.wgsl     # Waveform shader
└── Cargo.toml                    # Workspace root
```

## Key Components

### `apps/desktop/src/main.rs`
- Creates the winit window and drives the event loop
- Installs the macOS native app menu (`install_macos_app_menu`)
- Shows the native Settings dialog (`show_native_preferences_dialog`) — implemented with `NSPanel` + `NSTabView` (Audio / Recording / OSC tabs)
- Edit-time channel validation: `channelChanged:` (uniqueness) and `deviceChanged:` (clamping) ObjC action methods
- Loads/saves preferences as JSON via `AppPreferences`
- Calls into `crates/ui` and `crates/audio` via their public APIs

### `crates/audio` — Audio Engine
- **`AudioEngineCore`** (`engine_core.rs`): per-deck playback state, SoundTouch pitch-preserving time stretch, processing thread, device/channel reconfiguration
- **`WebAudioBackend`** (`engine_backend.rs`): persistent `web-audio-api` graph — 3-band EQ per deck, crossfader/gain, mic talkover, main/cue channel routing
- **`recorder.rs`**: WAV (hound) and OGG Vorbis (vorbis_rs) encoding on a dedicated thread
- **`beat_detector.rs`**: multi-peak correlation BPM detection
- **`list_output_devices()`**: enumerates cpal output devices for the Settings dialog

### `crates/ui` — Native Renderer
- **`renderer.rs`**: egui draw loop; renders console (decks, crossfader, waveform, EQ, meters), dispatches `UiAction` to the host
- **`ui_state.rs`**: shared types — `ConsoleVisualState`, `PreferencesState`, `AudioDeviceInfo`, `TitlebarState`
- **`lib.rs`**: C-ABI functions called from `apps/desktop`: `render_frame`, `set_console_state_raw`, `set_preferences_state_raw`, `handle_mouse_event`, etc.

## Key Features

### ✅ Dual Deck System
- Independent Deck A/B with crossfader (Pioneer-style constant power curve)
- Auto crossfade (2 s) with automatic deck switching
- Track preservation on stop

### ✅ Waveform Display
- Zoomed view (8-second window) + full-track view with click-to-seek
- wgpu shader-based rendering (`waveform.wgsl`)

### ✅ Audio Processing
- 44.1 kHz stereo via cpal (CoreAudio / WASAPI / ALSA)
- SoundTouch pitch-preserving tempo adjustment
- 3-band EQ with kill switches per deck
- Deck gain, level meters (15-segment LED), cue monitoring

### ✅ Microphone Input
- Ring buffer ingestion, talkover ducking (50% default)

### ✅ Dynamic Device Switching
- Runtime device/channel reconfiguration, hot-plug support
- Name-based device ID (stable across restarts)

### ✅ Session Recording
- WAV (lossless) and OGG Vorbis (compressed), format selectable in Settings

### ✅ OSC Broadcasting
- Real-time state broadcast to external controllers

### ✅ Native Settings Dialog (macOS)
- `NSPanel` with `NSTabView` — Audio / Recording / OSC
- Edit-time channel uniqueness and device-change clamping via ObjC target/action

## Development Guidelines

### Rust Patterns
```rust
// Calling the audio engine from the host
engine.set_deck_play(0, true);
engine.set_crossfader(0.5);
engine.configure_device(DeviceConfigCore { device_id: Some(name), .. });

// Pushing UI state from host to renderer
sujay_ui::set_console_state_raw(console_state);
sujay_ui::set_preferences_state_raw(prefs_state);
```

### macOS Native Dialog Pattern
```rust
// ObjC action method registered on NSPopUpButton
extern "C" fn settings_channel_changed(_this: &Object, _cmd: Sel, sender: id) {
    CHANNEL_CTX.with(|cell| {
        // enforce uniqueness across all 4 channel popups
    });
}
```

### State Flow
```
AudioEngineCore  →  EngineStateUpdate  →  main.rs event loop
                                              ↓
                                     ConsoleVisualState  →  sujay_ui::set_console_state_raw
                                              ↓
                                         egui renderer (renderer.rs)
```

## Build & Development

```bash
# Run (debug)
cargo run

# Lint
cargo clippy --all-targets

# Release build
cargo build --release

# macOS .app bundle
cargo bundle --release --manifest-path apps/desktop/Cargo.toml
```

## Code Style & Conventions
- Rust only — no TypeScript, no npm
- `cargo clippy --all-targets` must pass before commit
- `#![allow(unexpected_cfgs)]` in `apps/desktop/src/main.rs` suppresses `objc` macro cfg warnings
- Prefer `Arc<Mutex<T>>` for shared audio state; avoid blocking the audio callback
- macOS-specific code gated with `#[cfg(target_os = "macos")]`