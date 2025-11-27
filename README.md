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
- 🔄 **Crossfade** - Smooth transitions between tracks
- 🎧 **Cue Monitoring** - Independent headphone output per deck
- 💾 **Offline-First Design** - Fast startup with metadata caching
- 🤖 **MCP Integration** - Control via Model Context Protocol for AI automation

## Requirements

- Node.js 22 or higher
- macOS / Linux / Windows
- Suno AI account (for library features)

## Installation

```bash
# Clone the repository
git clone https://github.com/tnayuki/sujay.git
cd sujay

# Install dependencies
npm install
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

# Package build
npm run package

# Create installer
npm run make
```

## Architecture

### Audio Engine

- **Worker-Based**: Dedicated thread processing that doesn't block the main UI thread
- **MP3 Decoding**: `mpg123-decoder` (WASM)
- **Audio Output**: `naudiodon2` (PortAudio bindings)
- **Time Stretching**: High-quality tempo adjustment via SoundTouch
- **BPM Detection**: Multi-peak correlation algorithm

### Tech Stack

- **Runtime**: Electron + Node.js
- **Language**: TypeScript (strict mode)
- **UI**: React + Vite

## License

MIT License - See [LICENSE](LICENSE) file for details.