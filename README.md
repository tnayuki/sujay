# Sujay

**Sujay** is a macOS DJ application: a SwiftUI console over a Rust audio engine, with the local rekordbox library as its track source.

![License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- 🎛️ **Two decks and a crossfader** — play, cue, seek, beat loops from 1/16 to 32 beats
- 📚 **Rekordbox library** — the local rekordbox collection and playlists as a sortable table; loading a track brings its BPM, beat grid, hot and memory cues and 3-band waveform colours
- 📊 **Waveforms** — an 8-second zoom view per deck and a full-track view with click-to-seek
- 🎚️ **Mixer** — 3-band EQ kills, deck gain, 15-segment LED meters, cue monitoring, master tempo with pitch-preserving time stretch
- 🎤 **Microphone talkover** with music ducking
- 🔴 **Session recording** to WAV or OGG Vorbis
- 🔌 **Output device and channel routing** — main and cue on any pair of a multi-channel interface, switchable at runtime

## Requirements

- macOS 15 or later
- Xcode 26
- A Rust toolchain (stable)

## Build and run

```sh
git clone https://github.com/tnayuki/sujay.git
cd sujay
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
open ".build/DerivedData/Build/Products/Debug/Sujay Dev.app"
```

`sujay.xcodeproj` is the only build system. Its "Build Rust core" script phase runs `cargo build` for the Rust static library (`Vendor/build-rust.sh`), so a plain Xcode build is the whole build. Opening the project in Xcode and pressing ⌘R does the same.

A Release build is universal (arm64 + x86_64) and needs `rustup target add x86_64-apple-darwin` once.

## Usage

1. Start Sujay. The rekordbox library loads in the background from `~/Library/Pioneer/rekordbox/master.db` and reloads when rekordbox writes to it.
2. Load a track: drag a row from the library onto a deck, right-click it and choose the deck, or drop an audio file from Finder.
3. Play with the deck's button (Q for deck A, P for deck B), mix with the crossfader, set loops with the pads.
4. Settings (⌘,) hold the output device and the main / cue channel pairs, the recording folder and format, and OSC.

When the app is launched from Finder its log goes to `~/Library/Logs/Sujay/sujay.log`.

## Project layout

```
sujay/
├── sujay.xcodeproj              # the build; hand-authored, file-system-synchronized groups
├── Sources/
│   ├── Sujay/                   # SwiftUI console: decks, mixer, waveforms, library, settings
│   └── SujayCore/               # Swift wrapper over the C ABI
├── Resources/Info.plist
├── Vendor/
│   ├── SujayCore/include/       # sujay.h + module map, hand-written
│   └── build-rust.sh            # cargo build → .build/rust/<Configuration>/libsujay_ffi.a
├── Cargo.toml                   # Rust workspace
├── crates/
│   ├── audio/                   # engine: decks, SoundTouch time stretch, web-audio-api mix graph, recorder
│   ├── library/                 # rekordbox master.db and ANLZ reader (rbox)
│   ├── core/                    # host orchestration: preferences, decode, library load, engine state
│   └── ffi/                     # C ABI over core, built as a static library
└── docs/swift-migration-plan.md # the decisions behind this layout
```

## Architecture

```
SwiftUI console (Sources/Sujay)
     │  commands / per-frame snapshot / JSON on change / buffer copies
Swift wrapper (Sources/SujayCore) ── C ABI (Vendor/SujayCore/include/sujay.h)
     │
crates/ffi ── crates/core ──┬── crates/audio   AudioEngineCore (processing thread) → web-audio-api mix graph → CoreAudio
                            └── crates/library rekordbox master.db + ANLZ
```

State crosses the boundary at three rates: commands are plain functions; deck positions, peaks and flags are one POD struct read every frame; titles, cues, the library list and preferences are JSON read only when the core reports a change; waveforms and beat grids are copied out when a track loads.

The audio path is split into a per-deck stage (playback state, SoundTouch pitch-preserving time stretch) and a mix/routing stage — a persistent `web-audio-api` graph for the crossfader, deck gain, EQ kills, talkover ducking and main / cue channel mapping. Recording runs on its own thread.

## Development

```sh
cargo clippy --workspace --all-targets      # Rust
xcrun swift-format format -i -p -r Sources  # Swift, standard style (.swift-format)
git config core.hooksPath .githooks         # once per clone: lint staged Swift on commit
cargo run -p sujay-core --example smoke -- <audio file>   # drive the core without a window
```

Code, comments, documentation and commit messages are in English.

## License

MIT
