# Sujay — DJ application for macOS

## What it is

A SwiftUI console over an AVFoundation audio engine, all Swift. macOS only. The rekordbox library is the track source, read directly out of `master.db` and the ANLZ analysis files.

## Layout

```
sujay.xcodeproj               the build (hand-authored; edit project.pbxproj directly; Sources/ groups are file-system-synchronized)
Sources/Sujay/                SwiftUI console
Sources/SujayCore/Audio/      the engine: AudioEngine.swift (Engine: AVAudioSourceNode render, routing, commands, state), Deck.swift, TimeStretcher.swift (AVAudioUnitTimePitch via AUv2 render), Biquad.swift (kill EQ), MicInput.swift, Recorder.swift, AudioDevices.swift (HAL enumeration)
Sources/SujayCore/            AudioDecoder.swift (AVFoundation), Preferences.swift, SystemUsage.swift, Models.swift, Support.swift
Sources/SujayCore/Rekordbox/  MasterDB.swift (read-only SQLCipher connection), ANLZ.swift (binary analysis parser), RekordboxReader.swift (browse list, per-track analysis)
Sources/Sujay/Model.swift     ConsoleModel: the host orchestration — engine start, decode, library load/reload, rekordbox join, beat loops, per-frame state
Vendor/CSQLCipher.xcframework committed static SQLCipher (arm64 + x86_64); built by Vendor/build-sqlcipher.sh, by hand, not by the build
docs/swift-migration-plan.md  decisions and their reasons
```

## Engine

`Engine` (Sources/SujayCore/Audio) is used from the main thread; creating it starts CoreAudio, so `ConsoleModel.start` does that on a background thread and starts the frame timer when it returns.

- The render is one `AVAudioSourceNode` block: decks render under `lock.lockIfAvailable()` (a deck whose lock is held plays silence for that callback), then crossfader, talkover, cue mix, channel routing, recording push, and `publish()` of `EngineState` under a try-lock. No allocation on the render thread; scratch buffers are allocated once.
- `seek` and `setLoop` take fractions of the track; `setBeatLoop` takes seconds; beat grids are audio frames; the reported playhead is source frames consumed minus what the time-pitch unit still holds (its latency × rate).
- The time-pitch unit is rendered through `AudioUnitRender` with a render callback; the AUv3 `renderBlock` of this bridged unit fails with kAudioUnitErr_NoConnection.
- Rekordbox is two calls on `RekordboxReader`: `loadLibrary` (browse list, one pass over `master.db`, ~0.15 s for 1500 tracks) and `analysis` (one track's beat grid, cues, waveform colours). Both block; run them off the main thread.

## Rules that came from bugs

- What changes every frame (playhead, meters) is not `@Observable` and is not read by any SwiftUI body. A Canvas that read it re-evaluated sixty times a second, invalidated its size, and sent a layout pass through the `.fixedSize` parents to the root — 60 % of a core. The waveforms and meters are NSViews (`WaveformNSView`, `LevelMeterNSView`) that redraw on `ConsoleModel.addFrameListener` with CoreGraphics, anti-aliasing off, columns batched by colour, and skip a frame that would draw the same.
- Headless testing: `open --env SUJAY_AUTOPLAY=<audio file> "…/Sujay Dev.app"` loads the file on deck A and plays it after 3 s; measure with `ps -M -p <pid>` (per thread) or `top -pid`, at least 20 s after launch so the library load and decode are out of the number.

- Enumerate audio devices through the HAL property API only (`AudioDevices`). Creating an AudioUnit per device to ask, as cpal did, deadlocked inside CoreAudio on some machines.
- Beat loops are computed in `ConsoleModel.toggleLoop` from the track's beat grid in frames; the engine only gets seconds.
- Streaming entries in the rekordbox library (`spotify:track:…`) are not files; the library view shows local files only.
- `djmdContent.Length` is seconds and `BPM` is centi-BPM; one analysis lives in three files (`.DAT` beat grid and original cues, `.EXT` extended cues and colour waveforms, `.2EX` three-band waveforms), so all three are parsed together. A PWV5 colour column is a big-endian `u16`: red, green, blue three bits each from the top, then five bits of height.

## Build and check

```sh
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
xcrun swift-format lint --strict -r Sources          # pre-commit hook does this on staged files
```

Log output from a Finder-launched app: `~/Library/Logs/Sujay/sujay.log`.

## Style

- Swift: standard swift-format style, Swift 5 language mode, macOS 15 deployment target, SwiftUI with `@Observable`. Debug builds are `-O`, not `-Onone`: the audio render is Swift, and unoptimised it cost 17 % CPU for two playing decks against ~0 % optimised.
- Code, comments, docs and commit messages in English; commit subjects are one imperative line.
- Console look: system appearance and standard controls (`bordered` / `borderedProminent`, `GroupBox`, `Slider`, `Stepper`); custom drawing only for waveforms, meters and pads.
