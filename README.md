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
- 🍎 **AppleScript** — decks, mixer and the library as scriptable objects: load, play, loop, seek and read the console from `osascript`

## Requirements

- macOS 15 or later
- Xcode 26

## Build and run

```sh
git clone https://github.com/tnayuki/sujay.git
cd sujay
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
open ".build/DerivedData/Build/Products/Debug/Sujay Dev.app"
```

`sujay.xcodeproj` is the only build system and a plain Xcode build is the whole build — opening the project in Xcode and pressing ⌘R does the same. `Vendor/CSQLCipher.xcframework` is a committed binary, universal (arm64 + x86_64); `Vendor/build-sqlcipher.sh` rebuilds it from a pinned SQLCipher release and is run by hand, not by the build.

## Usage

1. Start Sujay. The rekordbox library loads in the background from `~/Library/Pioneer/rekordbox/master.db` and reloads when rekordbox writes to it.
2. Load a track: drag a row from the library onto a deck, right-click it and choose the deck, or drop an audio file from Finder.
3. Play with the deck's button (Q for deck A, P for deck B), mix with the crossfader, set loops with the pads.
4. Settings (⌘,) hold the output device and the main / cue channel pairs, the recording folder and format, and OSC.

When the app is launched from Finder its log goes to `~/Library/Logs/Sujay/sujay.log`.

## Scripting

The console is scriptable: the two decks, the mixer and the rekordbox library are objects, and a
script names what it acts on rather than reaching for the mouse.

```applescript
tell application "Sujay"
    load POSIX file "/Users/me/a.aiff" into deck 1  -- or a plain POSIX path; replies once the deck has the track
    load track "Aerodynamic" into deck 2         -- or name one from the library
    play deck 1
    set crossfader to 0                          -- 0 is deck A alone, 1 is deck B
    loop deck 1 beats 4                          -- from the beat before the playhead; 0 clears
    set position of deck 1 to 90                 -- seconds
    recall cue point "1" of deck 1
    set high kill of deck 2 to true
    get {title, bpm, position, duration} of deck 1
end tell
```

A deck carries `name`, `title`, `loaded`, `playing`, `position`, `duration`, `bpm`, `gain`,
`monitoring`, the three kills, `loop enabled`, `loop beats` and `level`, and holds the loaded
track's `cue point`s. The application carries `crossfader`, `master tempo`, `microphone enabled`,
`recording` and `library path`, and holds every `deck`, `track` and `playlist`. Open the dictionary
in Script Editor for the whole surface.

## Project layout

```
sujay/
├── sujay.xcodeproj              # the build; hand-authored, file-system-synchronized groups
├── Sources/
│   ├── Sujay/                   # SwiftUI console: decks, mixer, waveforms, library, settings, scripting
│   └── SujayCore/               # host core: the audio engine (Audio/), AVFoundation decode, preferences
│       └── Rekordbox/           # master.db over SQLCipher, and the ANLZ binary parser
├── Resources/
│   ├── Info.plist
│   ├── Sujay.sdef               # the AppleScript dictionary
│   ├── AppIcon.svg              # the icon's source; AppIcon-small.svg is the 16 / 32 px cut
│   ├── Assets.xcassets          # AppIcon.appiconset, rasterised from the SVGs
│   └── make-appicon.sh          # re-renders the appiconset; run by hand, not by the build
└── Vendor/
    ├── CSQLCipher.xcframework   # committed static build of SQLCipher
    └── build-sqlcipher.sh       # rebuilds it from a pinned release; run by hand
```

## Architecture

```
SwiftUI console (Sources/Sujay)
     │
Host core (Sources/SujayCore): ConsoleModel, AVFoundation decode, preferences, host stats
     │
Audio engine (Sources/SujayCore/Audio): one AVAudioSourceNode rendering two decks → device

Rekordbox reader (Sources/SujayCore/Rekordbox): master.db through SQLCipher, and the
ANLZ analysis files beside it — beat grid, cues, waveform colours
```

The engine renders everything itself inside one `AVAudioSourceNode`; the AVAudioEngine only carries the result to the device. Per deck: the playhead with sample-accurate loops, Apple's time-pitch unit (`AVAudioUnitTimePitch`, driven in pull mode through the AUv2 render API) for pitch-preserving tempo, a 3-band kill EQ of Butterworth biquads (250 Hz / 5 kHz), then gain and metering. The mix applies an equal-power crossfader, talkover ducking with the microphone from a second engine's input tap, a pre-fader cue mix, and writes main and cue to whichever device channels the settings name. Recording taps the main mix into an `AVAudioFile` (16-bit WAV or AAC).

The rekordbox reader opens `master.db` read-only — sujay never writes to the library rekordbox owns — and reads the browse list in one pass. A track's beat grid, cues and waveform colours are read on demand from the `.DAT`, `.EXT` and `.2EX` analysis files that sit together in one directory per track.

## Development

```sh
xcrun swift-format format -i -p -r Sources  # standard style (.swift-format)
git config core.hooksPath .githooks         # once per clone: lint staged Swift on commit
```

Code, comments, documentation and commit messages are in English.

## License

MIT
