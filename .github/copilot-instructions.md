# Sujay — DJ application for macOS

## What it is

A SwiftUI console over an AVFoundation audio engine, all Swift except the rekordbox reader, which is a Rust static library reached through a hand-written C ABI (two JSON calls). macOS only. The rekordbox library is the track source. #42 stage 2 removes the reader and with it Rust.

## Layout

```
sujay.xcodeproj               the build (hand-authored; edit project.pbxproj directly; Sources/ groups are file-system-synchronized)
Sources/Sujay/                SwiftUI console
Sources/SujayCore/Audio/      the engine: AudioEngine.swift (Engine: AVAudioSourceNode render, routing, commands, state), Deck.swift, TimeStretcher.swift (AVAudioUnitTimePitch via AUv2 render), Biquad.swift (kill EQ), MicInput.swift, Recorder.swift, AudioDevices.swift (HAL enumeration)
Sources/SujayCore/            Library.swift (C ABI wrapper), AudioDecoder.swift (AVFoundation), Preferences.swift, SystemUsage.swift, Models.swift, Support.swift
Sources/Sujay/Model.swift     ConsoleModel: the host orchestration — engine start, decode, library load/reload, rekordbox join, beat loops, per-frame state
Vendor/SujayCore/include/     sujay.h + module.modulemap — keep in step with crates/ffi/src/lib.rs
Vendor/build-rust.sh          script phase: cargo build → .build/rust/<Configuration>/libsujay_ffi.a
crates/library/               rekordbox master.db + ANLZ via rbox
crates/ffi/                   #[no_mangle] extern "C" over the reader; staticlib
docs/swift-migration-plan.md  decisions and their reasons
```

## Engine

`Engine` (Sources/SujayCore/Audio) is used from the main thread; creating it starts CoreAudio, so `ConsoleModel.start` does that on a background thread and starts the frame timer when it returns.

- The render is one `AVAudioSourceNode` block: decks render under `lock.lockIfAvailable()` (a deck whose lock is held plays silence for that callback), then crossfader, talkover, cue mix, channel routing, recording push, and `publish()` of `EngineState` under a try-lock. No allocation on the render thread; scratch buffers are allocated once.
- `seek` and `setLoop` take fractions of the track; `setBeatLoop` takes seconds; beat grids are audio frames; the reported playhead is source frames consumed minus what the time-pitch unit still holds (its latency × rate).
- The time-pitch unit is rendered through `AudioUnitRender` with a render callback; the AUv3 `renderBlock` of this bridged unit fails with kAudioUnitErr_NoConnection.
- Rekordbox is two JSON calls: `sujay_library_load_json` (browse list) and `sujay_library_track_analysis_json` (beat grid, cues, waveform colours). Keys are snake_case; `JSON.decoder` converts.

## Rules that came from bugs

- Enumerate audio devices through the HAL property API only (`AudioDevices`). Creating an AudioUnit per device to ask, as cpal did, deadlocked inside CoreAudio on some machines.
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
- Swift: standard swift-format style, Swift 5 language mode, macOS 15 deployment target, SwiftUI with `@Observable`. Debug builds are `-O`, not `-Onone`: the audio render is Swift, and unoptimised it cost 17 % CPU for two playing decks against ~0 % optimised.
- Code, comments, docs and commit messages in English; commit subjects are one imperative line.
- Console look: system appearance and standard controls (`bordered` / `borderedProminent`, `GroupBox`, `Slider`, `Stepper`); custom drawing only for waveforms, meters and pads.
