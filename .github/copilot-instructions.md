# Sujay — DJ application for macOS

## What it is

A SwiftUI console over a Rust audio engine. The Swift app is the only host; the Rust side is a static library reached through a hand-written C ABI. macOS only. The rekordbox library is the track source.

## Layout

```
sujay.xcodeproj               the build (hand-authored; edit project.pbxproj directly; Sources/ groups are file-system-synchronized)
Sources/Sujay/                SwiftUI console
Sources/SujayCore/            Engine.swift (C ABI wrapper), Models.swift (Codable mirrors of the JSON)
Vendor/SujayCore/include/     sujay.h + module.modulemap — keep in step with crates/ffi/src/lib.rs
Vendor/build-rust.sh          script phase: cargo build → .build/rust/<Configuration>/libsujay_ffi.a
crates/audio/                 AudioEngineCore, WebAudioBackend (web-audio-api graph), decoder (symphonia), recorder
crates/library/               rekordbox master.db + ANLZ via rbox
crates/core/                  Core: preferences, background decode, library load, engine state → console state
crates/ffi/                   #[no_mangle] extern "C" over Core; staticlib
docs/swift-migration-plan.md  decisions and their reasons
```

## Boundary

`Core` (crates/core) is single-threaded by contract and driven from the Swift main thread through `crates/ffi`:

- **Commands** are plain C functions (`sujay_core_play`, `sujay_core_set_eq`, …), one per action.
- **Fast state** is `sujay_core_snapshot` into a POD struct (`SujaySnapshot`), read every frame.
- **Slow state** — titles, cues, the library list, preferences — is JSON (`sujay_core_*_json`), read only when `sujay_core_tick` reports a change. JSON keys are snake_case; the Swift decoder converts.
- **Bulk buffers** — waveform, colours, beat grid — are copied into caller memory on deck load.

Engine start enumerates CoreAudio devices; the Swift model runs it off the main thread and starts the frame timer when it returns.

## Rules that came from bugs

- Do not enumerate audio devices through cpal or web-audio-api. Both create an AudioUnit per device to answer, which deadlocks inside CoreAudio on some machines. `list_output_devices` reads the HAL property API; the mix backend gets a device name only for a non-default device (#38 tracks removing that case too).
- The beat grid lives in `crates/core`, not in the UI. Beat loops and slices are computed from it there.
- Streaming entries in the rekordbox library (`spotify:track:…`) are not files; the library view shows local files only.

## Build and check

```sh
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
cargo clippy --workspace --all-targets
xcrun swift-format lint --strict -r Sources          # pre-commit hook does this on staged files
cargo run -p sujay-core --example smoke -- <audio>    # headless end-to-end check of the core
```

Log output from a Finder-launched app: `~/Library/Logs/Sujay/sujay.log`.

## Style

- Rust: 2-space indent (`rustfmt.toml` per crate), `cargo clippy` clean. Prefer `parking_lot::Mutex`; never block the audio callback.
- Swift: standard swift-format style, Swift 5 language mode, macOS 15 deployment target, SwiftUI with `@Observable`.
- Code, comments, docs and commit messages in English; commit subjects are one imperative line.
- Console look: system appearance and standard controls (`bordered` / `borderedProminent`, `GroupBox`, `Slider`, `Stepper`); custom drawing only for waveforms, meters and pads.
