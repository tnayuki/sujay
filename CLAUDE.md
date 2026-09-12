# Sujay — DJ application for macOS

This file holds what an agent needs before touching the code: the layout, the engine, the
rules that came from bugs, and how to build and check. The README describes only what
currently works; the reasoning behind a decision lives in git history.

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
Sources/Sujay/Scripting.swift  the AppleScript verbs; SDScripting.swift is the object model (the SD… proxies)
Resources/Sujay.sdef          the scripting dictionary, copied into the bundle by the resources phase
Resources/AppIcon.svg         the app icon's source (AppIcon-small.svg is a simplified cut for 16 and 32 px), rasterised by make-appicon.sh into Assets.xcassets/AppIcon.appiconset; run by hand, not by the build
Vendor/CSQLCipher.xcframework committed static SQLCipher (arm64 + x86_64); built by Vendor/build-sqlcipher.sh, by hand, not by the build
```

## Engine

`Engine` (Sources/SujayCore/Audio) is used from the main thread; creating it starts CoreAudio, so `ConsoleModel.start` does that on a background thread and starts the frame timer when it returns.

- The render is one `AVAudioSourceNode` block: decks render under `lock.lockIfAvailable()` (a deck whose lock is held plays silence for that callback), then crossfader, talkover, cue mix, channel routing, recording push, and `publish()` of `EngineState` under a try-lock. No allocation on the render thread; scratch buffers are allocated once.
- `seek` and `setLoop` take fractions of the track; `setBeatLoop` takes seconds; beat grids are audio frames; the reported playhead is source frames consumed minus what the time-pitch unit still holds (its latency × rate).
- The time-pitch unit is rendered through `AudioUnitRender` with a render callback; the AUv3 `renderBlock` of this bridged unit fails with kAudioUnitErr_NoConnection.
- Rekordbox is two calls on `RekordboxReader`: `loadLibrary` (browse list, one pass over `master.db`, ~0.15 s for 1500 tracks) and `analysis` (one track's beat grid, cues, waveform colours). Both block; run them off the main thread.

## Scripting

The console is scriptable — `Resources/Sujay.sdef` is the dictionary, `SDScripting.swift` the object
model (application → deck → cue point, with the library's tracks and playlists on the application),
`Scripting.swift` the verbs.

- The `SD…` proxies hold an identity only — deck index, cue label, rekordbox id — and re-resolve the
  model on every access, so one that outlives what it names reads empty rather than stale. Their
  `@objc(SD…)` runtime names are what the sdef binds to; a mangled Swift name leaves every property
  `missing value`.
- A verb whose receiver is its object (`play deck 1`) is a `responds-to` method on the proxy, not an
  `NSScriptCommand` subclass: with a `<cocoa class>` the subclass owns the dispatch and the object
  direct-parameter never reaches the proxy. `load` names its deck with `into`, because its own
  direct parameter is what to load, and suspends the command until the deck has the track so the
  next line can play it.

## Rules that came from bugs

- What changes every frame (playhead, meters) is not `@Observable` and is not read by any SwiftUI body. A Canvas that read it re-evaluated sixty times a second, invalidated its size, and sent a layout pass through the `.fixedSize` parents to the root — 60 % of a core. The waveforms and meters are NSViews (`WaveformNSView`, `LevelMeterNSView`) that redraw on `ConsoleModel.addFrameListener` with CoreGraphics, anti-aliasing off, columns batched by colour, and skip a frame that would draw the same.
- Headless testing: open the app, then `osascript -e 'tell application "Sujay Dev" to load "<audio file>" into deck 1' -e 'tell application "Sujay Dev" to play deck 1'` — the load replies once the deck has the track. Measure with `ps -M -p <pid>` (per thread) or `top -pid`, at least 20 s after launch so the library load and decode are out of the number.
- `@NSApplicationDelegateAdaptor` hands the `App` struct a delegate that is not the one `NSApp` keeps: a model assigned to it from a view's `onAppear` is invisible to `NSApp.delegate`, which is where scripting and termination look. The console is reached through `ConsoleModel.current` instead, set in `start()`.
- `ConsoleModel`'s commands write what they set into the published state as well as into the engine. The engine publishes on its render callback and the frame timer copies that up to a frame later, so anything reading straight back — a script above all — would see the old value. Stopping a recording is the exception: the writer thread is still draining the ring, and the engine rightly goes on reporting a recording until it has.
- A `file` in a reply — `location` of a track — makes the Apple Event manager issue a sandbox extension for it, and that blocks on the app's access to the folder: without the grant for `~/Music`, reading `location of track 1` wedges the main thread inside `AEProcessAppleEvent` until the prompt is answered. A text property (`POSIX path`) never does. It is the same grant a decode needs, so a console that can play its library can answer this too.
- Checking the scripting surface from a terminal needs an Automation grant for the app; without one an Apple event does not fail, it times out (-1712). An event a process sends to itself is exempt, so a temporary hook that runs `NSAppleScript` against the app itself checks the whole surface — off the main thread, or a suspended command (`load`) can never resume. Opening a file under `~/Music` needs its own grant, so decode a file from `/tmp` when testing.

- Enumerate audio devices through the HAL property API only (`AudioDevices`). Creating an AudioUnit per device to ask, as cpal did, deadlocked inside CoreAudio on some machines.
- Beat loops are computed in `ConsoleModel.toggleLoop` from the track's beat grid in frames; the engine only gets seconds.
- Streaming entries in the rekordbox library (`spotify:track:…`) are not files; the library view shows local files only.
- `djmdContent.Length` is seconds and `BPM` is centi-BPM; one analysis lives in three files (`.DAT` beat grid and original cues, `.EXT` extended cues and colour waveforms, `.2EX` three-band waveforms), so all three are parsed together. A PWV5 colour column is a big-endian `u16`: red, green, blue three bits each from the top, then five bits of height.

## Build and check

```sh
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
xcrun swift-format lint --strict -r Sources          # pre-commit hook does this on staged files
osascript -e 'tell application "Sujay Dev" to sujay status'   # both decks and the mixer, one line each
```

Log output from a Finder-launched app: `~/Library/Logs/Sujay/sujay.log`.

The app icon is drawn in `Resources/AppIcon.svg`; after editing it run `Resources/make-appicon.sh`
(needs `rsvg-convert`) to re-render the appiconset. Every slot gets its own PNG even where two
slots are the same number of pixels — sharing a filename makes actool drop sizes.

## Style

- Swift: standard swift-format style, Swift 5 language mode, macOS 15 deployment target, SwiftUI with `@Observable`. Debug builds are `-O`, not `-Onone`: the audio render is Swift, and unoptimised it cost 17 % CPU for two playing decks against ~0 % optimised.
- Code, comments, docs and commit messages in English; commit subjects are one imperative line.
- Console look: system appearance and standard controls (`bordered` / `borderedProminent`, `GroupBox`, `Slider`, `Stepper`); custom drawing only for waveforms, meters and pads.
