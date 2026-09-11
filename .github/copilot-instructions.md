# Sujay — DJ application for macOS

## What it is

A SwiftUI console over a Rust audio engine. The Swift app is the only host and owns everything but the engine and the rekordbox reader, which are a static library reached through a hand-written C ABI. macOS only. The rekordbox library is the track source. Rust is being removed in stages (#42): the reader next, then the engine.

## Layout

```
sujay.xcodeproj               the build (hand-authored; edit project.pbxproj directly; Sources/ groups are file-system-synchronized)
Sources/Sujay/                SwiftUI console
Sources/SujayCore/            Engine.swift, Library.swift (C ABI wrappers), AudioDecoder.swift (AVFoundation), Preferences.swift, SystemUsage.swift, Models.swift
Sources/Sujay/Model.swift     ConsoleModel: the host orchestration — engine start, decode, library load/reload, rekordbox join, beat loops, per-frame state
Vendor/SujayCore/include/     sujay.h + module.modulemap — keep in step with crates/ffi/src/lib.rs
Vendor/build-rust.sh          script phase: cargo build → .build/rust/<Configuration>/libsujay_ffi.a
crates/audio/                 AudioEngineCore, WebAudioBackend (web-audio-api graph), decoder (symphonia), recorder
crates/library/               rekordbox master.db + ANLZ via rbox
crates/ffi/                   #[no_mangle] extern "C" over AudioEngineCore and the reader; staticlib
docs/swift-migration-plan.md  decisions and their reasons
```

## Boundary

The engine handle (`SujayEngine`) is used from the main thread; creating it enumerates CoreAudio devices, so `ConsoleModel.start` does that on a background thread and starts the frame timer when it returns.

- **Commands** are plain C functions (`sujay_engine_play`, `sujay_engine_set_eq`, …). `seek` and `set_loop` take fractions of the track; `set_beat_loop` takes seconds; beat grids are audio frames.
- **State** is `sujay_engine_state` into a POD struct, read every frame; Swift keeps titles, cues and waveforms itself.
- **Loading** is `sujay_engine_load_track` with interleaved stereo PCM decoded by AVFoundation in Swift.
- **Rekordbox** is two JSON calls: `sujay_library_load_json` (browse list) and `sujay_library_track_analysis_json` (beat grid, cues, waveform colours). Keys are snake_case; `JSON.decoder` converts.

## Rules that came from bugs

- Do not enumerate audio devices through cpal or web-audio-api. Both create an AudioUnit per device to answer, which deadlocks inside CoreAudio on some machines. `list_output_devices` reads the HAL property API; the mix backend gets a device name only for a non-default device (#38 tracks removing that case too).
- Beat loops are computed in `ConsoleModel.toggleLoop` from the track's beat grid in frames; the engine only gets seconds.
- Streaming entries in the rekordbox library (`spotify:track:…`) are not files; the library view shows local files only.

## Build and check

```sh
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
cargo clippy --workspace --all-targets
xcrun swift-format lint --strict -r Sources          # pre-commit hook does this on staged files
```

Log output from a Finder-launched app: `~/Library/Logs/Sujay/sujay.log`.

## Style

- Rust: 2-space indent (`rustfmt.toml` per crate), `cargo clippy` clean. Prefer `parking_lot::Mutex`; never block the audio callback.
- Swift: standard swift-format style, Swift 5 language mode, macOS 15 deployment target, SwiftUI with `@Observable`.
- Code, comments, docs and commit messages in English; commit subjects are one imperative line.
- Console look: system appearance and standard controls (`bordered` / `borderedProminent`, `GroupBox`, `Slider`, `Stepper`); custom drawing only for waveforms, meters and pads.
