# Sujay

**Sujay** is an AI-powered DJ application integrated with Suno AI. It provides a complete DJ experience with dual decks, crossfader, and waveform visualization.

![License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- 🎛️ **Dual Deck System** - Two independent decks with crossfader
- 🎵 **Suno AI Integration** - Direct library management for AI-generated music
- 📊 **Professional Waveform Display** - Zoom view and full track view
- 🎚️ **Advanced Audio Processing**
  - Automatic BPM detection and tempo sync
  - 3-band EQ (Low/Mid/High) kill switches
  - Level meters (15-segment LED display)
  - Deck gain control
- 🎤 **Microphone Input** - Talkover with automatic music ducking
- 🔄 **Crossfade** - Smooth transitions between tracks
- 🎧 **Cue Monitoring** - Independent headphone output per deck
- 🔌 **Dynamic Device Switching** - Runtime device switching with hot-plug support
- 🔴 **Session Recording** - Record mixes to WAV or OGG Vorbis files
- 💾 **Offline-First Design** - Fast startup with metadata caching
- 🤖 **MCP Integration** - Control via Model Context Protocol for AI automation

## Requirements

- Node.js 22 or higher
- Rust toolchain (for building native audio module)
- macOS / Linux / Windows
- Suno AI account (for library features)

## Installation

```bash
# Clone the repository
git clone https://github.com/tnayuki/sujay.git
cd sujay

# Install dependencies
npm install

# Build Rust audio engine (required for first time)
npm run build
```

## Setup

### Suno AI Authentication

1. Log in to [Suno AI](https://suno.com) in your browser
2. Open Developer Tools > Application > Cookies > `https://suno.com`
3. Copy all cookies in the format `__client=<value>; __session=<value>; ...`
4. Launch the application and open Preferences (⌘,)
5. Paste the copied cookie string into the "Suno Session Cookie" field and save

## Usage

```bash
# Start application
npm start
```

### MCP (Model Context Protocol) Integration

Sujay includes a built-in MCP server that allows AI assistants to control the DJ application. You can use tools like GitHub Copilot or custom MCP clients to:

- Load tracks to decks
- Control playback and crossfading
- Adjust EQ and gain settings
- Monitor deck status and positions

MCP server endpoint: `http://localhost:8888/mcp`

## Development

```bash
# Lint check
npm run lint

# Rebuild Rust audio engine after changes
npm run build

# Package build (run from app/ directory)
cd app && npm run package

# Create installer
cd app && npm run make
```

## Project Structure

```
sujay/
├── app/                  # Electron application
│   ├── src/              # Application source code
│   ├── package.json      # App dependencies
│   └── forge.config.js   # Electron Forge config
├── packages/
│   └── audio/            # Native audio engine (Rust + cpal + SoundTouch + web-audio-api)
├── patches/              # npm package patches
└── package.json          # Workspace root
```

## Architecture

### Audio Engine (Rust)

The audio path is implemented in Rust and split into two stages:

- **Deck Processing Stage** - Per-deck playback state and SoundTouch time stretching (pitch-preserving)
- **Mix/Routing Stage** - Persistent `web-audio-api` backend graph for crossfader, deck gain, 3-band EQ kill, main/cue routing, and mic talkover mix
- **Microphone Input** - Ring buffer ingestion with ducking-aware mix integration
- **Session Recording** - WAV (lossless) and OGG Vorbis (compressed) encoding
- **Dynamic Device Switching** - Runtime device/channel reconfiguration with hot-plug handling
- **Thread Priority** - Real-time priority for audio processing thread
- **Audio I/O** - Cross-platform output/input via cpal (CoreAudio/WASAPI/ALSA)

web-audio-api backend notes (current behavior):

- Backend I/O mapping is configured from runtime channel config (main/cue).
- Context initialization is reused across renders and recreated on device/channel reconfigure.
- Panic-safe fallback remains in place inside backend render to avoid audio thread hard-failure.

Current node graph (mix/routing stage):

```
Deck A MediaStreamTrackSource -> DeckA EQ(3-band + kills) -> Gain(A) --+
                                                                      |
Deck B MediaStreamTrackSource -> DeckB EQ(3-band + kills) -> Gain(B) --+-> MusicBus Gain (talkover attenuation) --+
                                                                                                                  |
Mic MediaStreamTrackSource -> Gain(Mic) --------------------------------------------------------------------------+-> MasterBus Gain
                                                                                                                          |
                                                                                                                          +-> ChannelSplitter(2) -> Main ChannelMerger(output_channels) -> Destination
                                                                                                                          |
CueMix MediaStreamTrackSource (A/B pre-fader mix, when cue enabled) -> ChannelSplitter(2) -> Main ChannelMerger(output_channels)
```

Node roles:

- `MediaStreamTrackSource` - persistent streaming source nodes for deck A/B, mic, and cue mix
- `Deck EQ` - per-deck 3-band biquad chain with low/mid/high kill switches
- `Gain(A/B)` - crossfader curve and deck gain applied to each deck signal
- `MusicBus Gain` - talkover ducking amount when mic is enabled
- `Gain(Mic)` - microphone level control
- `MasterBus Gain` - final summing node before output routing
- `ChannelSplitter(2)` - split stereo bus into L/R channels
- `ChannelMerger(output_channels)` - map main/cue to runtime device channel layout

### Worker Architecture

```
Main Process → Audio Worker
                ↓
      Rust AudioEngine (processing thread)
          ↓            ↓                ↓
  Deck Stretch+State   web-audio-api Mix+EQ   Recording Thread (optional)
```

Auxiliary: OSC Manager broadcasts mixer/deck state for external controllers.

### Tech Stack

- **Runtime**: Electron + Node.js
- **Language**: TypeScript (strict mode) + Rust
- **UI**: React + Vite
- **Native Bindings**: napi-rs
- **Audio Graph Backend**: web-audio-api
- **Monorepo**: npm workspaces

## License

MIT License - See [LICENSE](LICENSE) file for details.