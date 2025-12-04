# Sujay

**Sujay** is a AI-powered DJ application integrated with Suno AI. It provides a complete DJ experience with dual decks, crossfader, and waveform visualization.

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
│   └── audio/            # Native audio engine (Rust + cpal + SoundTouch)
├── patches/              # npm package patches
└── package.json          # Workspace root
```

## Architecture

### Audio Engine (Rust)

The audio engine is fully implemented in Rust for maximum performance:

- **Dual Deck Playback** - Independent deck management with crossfader
- **Time Stretching** - SoundTouch-based tempo adjustment with pitch preservation
- **3-Band EQ** - Biquad filter implementation with kill switches
- **Microphone Input** - Ring buffer with talkover ducking
- **Session Recording** - WAV (lossless) and OGG Vorbis (compressed) encoding
- **Dynamic Device Switching** - Runtime device/channel configuration with seamless hot-plug support
- **Thread Priority** - Real-time thread priority for low-latency audio
- **Audio I/O** - Cross-platform audio via cpal (CoreAudio/WASAPI/ALSA)

### Worker Architecture

```
Main Process → Audio Worker → Decode Worker
                ↓                ↓
         Rust AudioEngine   MP3 → PCM + BPM
                ↓
         Recording Writer (optional)
```

### Tech Stack

- **Runtime**: Electron + Node.js
- **Language**: TypeScript (strict mode) + Rust
- **UI**: React + Vite
- **Native Bindings**: napi-rs
- **Monorepo**: npm workspaces

## License

MIT License - See [LICENSE](LICENSE) file for details.