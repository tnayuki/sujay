# Sujay

**Sujay** is a macOS DJ application in Swift: a SwiftUI console over an AVFoundation audio engine, with the local rekordbox library as its track source.

![License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- 🎛️ **Two decks and a crossfader** — play, cue, seek, beat loops from 1/16 to 32 beats
- 📚 **Rekordbox library** — the local rekordbox collection and playlists as a sortable table; loading a track brings its BPM, beat grid, hot and memory cues and 3-band waveform colours
- 📊 **Waveforms** — an 8-second zoom view per deck and a full-track view with click-to-seek
- 🎚️ **Mixer** — 3-band EQ kills, deck gain, 15-segment LED meters, cue monitoring, master tempo with pitch-preserving time stretch
- 🎤 **Microphone talkover** with music ducking
- 🔴 **Session recording** to WAV or AAC
- 🔌 **Output device and channel routing** — main and cue on any pair of a multi-channel interface, switchable at runtime

## Requirements

- macOS 15 or later
- Xcode 26
- A Rust toolchain (stable), for the rekordbox reader (until #42 stage 2)

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
│   └── SujayCore/               # host core: the audio engine (Audio/), AVFoundation decode, rekordbox wrapper, preferences
├── Resources/Info.plist
├── Vendor/
│   ├── SujayCore/include/       # sujay.h + module map, hand-written
│   └── build-rust.sh            # cargo build → .build/rust/<Configuration>/libsujay_ffi.a
├── Cargo.toml                   # Rust workspace
├── crates/
│   ├── library/                 # rekordbox master.db and ANLZ reader (rbox)
│   └── ffi/                     # C ABI over the reader, built as a static library
└── docs/swift-migration-plan.md # the decisions behind this layout
```

## Architecture

```
SwiftUI console (Sources/Sujay)
     │
Host core (Sources/SujayCore): ConsoleModel, AVFoundation decode, preferences, host stats
     │
Audio engine (Sources/SujayCore/Audio): one AVAudioSourceNode rendering two decks → device
     │  C ABI (Vendor/SujayCore/include/sujay.h), two JSON calls
crates/ffi ── crates/library   rekordbox master.db + ANLZ
```

The engine renders everything itself inside one `AVAudioSourceNode`; the AVAudioEngine only carries the result to the device. Per deck: the playhead with sample-accurate loops, Apple's time-pitch unit (`AVAudioUnitTimePitch`, driven in pull mode through the AUv2 render API) for pitch-preserving tempo, a 3-band kill EQ of Butterworth biquads (250 Hz / 5 kHz), then gain and metering. The mix applies an equal-power crossfader, talkover ducking with the microphone from a second engine's input tap, a pre-fader cue mix, and writes main and cue to whichever device channels the settings name. Recording taps the main mix into an `AVAudioFile` (16-bit WAV or AAC).

The only Rust left is the rekordbox reader, reached through two JSON calls (the browse list, one track's analysis); it goes in the next stage of #42.

## Development

```sh
cargo clippy --workspace --all-targets      # Rust
xcrun swift-format format -i -p -r Sources  # Swift, standard style (.swift-format)
git config core.hooksPath .githooks         # once per clone: lint staged Swift on commit
```

Code, comments, documentation and commit messages are in English.

## License

MIT
