# Sujay - AI DJ Application

## Project Overview
Sujay is a professional AI-powered DJ application built with Electron, TypeScript, and React. It integrates with Suno AI for music generation and provides a complete DJ experience with dual decks, crossfading, and waveform visualization.

## Current Architecture

### Core Technologies
- **Runtime**: Node.js 22+ with Electron
- **Languages**: TypeScript (strict mode) + Rust
- **UI Framework**: React with Vite build system
- **Audio Engine**: Rust-based architecture (`@sujay/audio`) with:
  - **Audio I/O**: cpal (cross-platform: CoreAudio/WASAPI/ALSA)
  - **Time Stretching**: SoundTouch (C++ via soundtouch crate)
  - **3-Band EQ**: Biquad filter implementation with kill switches
  - **Microphone Input**: Ring buffer with talkover ducking
  - **Thread Priority**: Real-time priority via thread-priority crate
  - **MP3 Decoding**: `mpg123-decoder` (WASM-based, in decode worker)
  - **BPM Detection**: Custom algorithm with multi-peak correlation
  - **Recording**: WAV and OGG Vorbis file recording to disk (Rust + TypeScript worker)
  - **OSC Broadcasting**: Real-time state broadcasting for external controllers

### Worker Architecture
```
Main Process → Audio Worker → Decode Worker
                ↓                ↓
         Rust AudioEngine   MP3 → PCM + BPM
                ↓
         Recording Writer (optional)
```

**Audio Workers**:
- `src/workers/audio-worker.ts`: Bridge to Rust AudioEngine, handles IPC
- `src/workers/audio-decode-worker.ts`: MP3 decoding and BPM detection (separate thread)
- `src/workers/recording-writer.ts`: WAV/OGG file writing in separate thread
- `src/workers/osc-manager.ts`: OSC message broadcasting

**Rust Audio Engine** (`packages/audio/src/`):
- `audio_engine.rs`: Core DJ mixing engine (dual decks, crossfader, mic input)
- `eq_processor.rs`: 3-band EQ with biquad filters
- `recorder.rs`: Recording thread with WAV (hound) and OGG (vorbis_rs) encoders
- `lib.rs`: napi-rs bindings and device enumeration

### Key Features Implemented

#### ✅ Dual Deck System
- Independent Deck A/B with crossfader support
- Auto crossfade (2 seconds) with automatic deck switching
- Manual crossfader control with Pioneer-style constant power curve
- Track preservation on stop (decks remain loaded)

#### ✅ Advanced Waveform Display
- **Zoomed View**: 8-second window with playback position interpolation
- **Full Track View**: Click-to-seek functionality
- Real-time position updates with seek operation detection
- Canvas-based rendering optimized for performance

#### ✅ Professional Audio Processing
- 44.1kHz stereo output via cpal (cross-platform Rust audio library)
- Transferable object architecture for zero-copy audio data
- Automatic BPM detection with tempo-sync playback
- 3-band EQ with kill switches (Low/Mid/High) per deck
- Deck gain control (0-100%)
- Level meters with 15-segment LED display
- Cue monitoring with independent headphone output

#### ✅ Microphone Input
- Ring buffer for low-latency input
- Talkover with automatic music ducking (50% default)
- Level meter display (always visible, regardless of enabled state)
- LED-style UI indicator matching recording button

#### ✅ Dynamic Audio Device Switching
- Hot-plug support for audio devices
- Runtime device switching via `configure_device()` method
- Name-based device identification (stable across restarts)
- Independent main output and cue/headphone channel configuration
- Seamless stream recreation without audio glitches
- Device preference persistence

#### ✅ Library Management
- Suno AI integration with workspace support
- Offline-first metadata caching (JSON-based)
- Track structure analysis for optimal DJ mixing
- Right-click context menu for deck loading
- Real-time generation status and download progress

#### ✅ Recording & Broadcasting
- Session recording to WAV or OGG Vorbis files
- Format selection in preferences UI (WAV lossless or OGG compressed)
- OSC broadcasting for external controllers
- Real-time state synchronization

#### ✅ MCP Integration
- Built-in Model Context Protocol server
- HTTP endpoint on `http://localhost:8888/mcp`
- AI automation support for playback, mixing, and track management
- Integration with GitHub Copilot and other MCP clients

## Code Organization

### Directory Structure
```
sujay/
├── app/                      # Electron application
│   ├── src/
│   │   ├── workers/          # Audio processing (separate threads)
│   │   │   ├── audio-worker.ts      # Bridge to Rust AudioEngine
│   │   │   ├── audio-worker-types.ts # Worker message types
│   │   │   ├── audio-decode-worker.ts # MP3 decoding + BPM detection
│   │   │   ├── bpm-detector.ts       # BPM detection algorithm
│   │   │   ├── recording-writer.ts   # WAV file recording
│   │   │   └── osc-manager.ts        # OSC broadcasting
│   │   ├── core/             # Business logic (library, metadata)
│   │   │   ├── library-manager.ts
│   │   │   ├── metadata-cache.ts
│   │   │   ├── structure-cache.ts
│   │   │   └── controllers/
│   │   │       └── mcp-controller.ts
│   │   ├── main/             # Main process modules
│   │   │   └── mcp-server.ts
│   │   ├── renderer/         # React UI components
│   │   │   ├── App.tsx
│   │   │   └── components/
│   │   ├── types/            # TypeScript definitions
│   │   ├── main.ts           # Electron main process
│   │   ├── preload.ts        # IPC bridge
│   │   ├── suno-api.ts       # Suno AI client
│   │   └── types.ts          # Shared type definitions
│   ├── package.json          # App dependencies
│   └── forge.config.js       # Electron Forge config
├── packages/
│   └── audio/                 # Rust audio engine (cpal + napi-rs)
│       ├── src/
│       │   ├── lib.rs         # napi-rs bindings, device enumeration
│       │   ├── audio_engine.rs # Core DJ engine (decks, crossfader, mic)
│       │   ├── recorder.rs     # Recording thread (WAV/OGG encoders)
│       │   └── eq_processor.rs # 3-band EQ with biquad filters
│       ├── Cargo.toml         # Rust dependencies
│       └── index.d.ts         # TypeScript declarations
├── patches/                   # npm package patches
└── package.json               # Workspace root
```

### Key Components

**Rust Audio Engine** (`packages/audio/src/audio_engine.rs`):
- Core DJ mixing engine with dual decks and crossfader
- Real-time audio processing on dedicated thread
- Dynamic device switching via `configure_device()` method
- Name-based device identification for stable references
- Microphone input with talkover ducking
- 3-band EQ with biquad filters
- Cue monitoring with independent headphone output
- Ring buffers for low-latency sample delivery

**Audio Worker Bridge** (`app/src/workers/audio-worker.ts`):
- Bridge between TypeScript and Rust AudioEngine
- State conversion and IPC handling
- Waveform generation from decoded PCM
- OSC state broadcasting

**MCP Server** (`app/src/main/mcp-server.ts`):
- Express HTTP server with MCP SDK integration
- Tools for track loading, playback control, EQ, crossfading
- State monitoring (deck info, crossfader, master tempo)
- Integration with MCPController for business logic

**MCPController** (`app/src/core/controllers/mcp-controller.ts`):
- Bridge between MCP server and LibraryManager/AudioWorker
- Caches audio state for fast responses
- Handles track structure analysis for DJ mixing

**Console Component** (`app/src/renderer/components/Console.tsx`):
- Deck controls with play/stop buttons
- Crossfader with mouse drag support
- Waveform displays (zoom + full)
- Level meters and tempo display
- EQ kill switches per deck
- Deck gain sliders
- Cue monitoring buttons

**Library Component** (`app/src/renderer/components/Library.tsx`):
- Track table with status indicators
- Generation interface
- Workspace/filter controls
- Native context menu integration

## Development Guidelines

### Rust Audio Engine Patterns
```rust
// Dynamic device switching
pub fn configure_device(&mut self, config: DeviceConfig) {
    // Stops existing stream, reconfigures, and recreates
    let mut stream_guard = self.stream.lock().unwrap();
    *stream_guard = None; // Drop old stream
    // ... configure new device and channels ...
    *stream_guard = Some(new_stream);
}

// Microphone input with ring buffer
let mic_state = Arc::new(Mutex::new(MicrophoneState {
    enabled: false,
    gain: 1.0,
    talkover_ducking: 0.5,
    input_buffer: VecDeque::with_capacity(4096),
    peak: 0.0,
}));

// Apply talkover ducking when mic is enabled
fn apply_mic_talkover(&self, left: &mut f32, right: &mut f32) {
    if mic_state.enabled && !mic_state.input_buffer.is_empty() {
        *left *= mic_state.talkover_ducking;
        *right *= mic_state.talkover_ducking;
    }
}
```

### Audio Worker Patterns
```typescript
// Use transferable objects for audio data
parentPort.postMessage(result, [pcmBuffer, monoBuffer]);

// State conversion from Rust to TypeScript
const state: RustAudioEngineStateUpdate = {
    micAvailable: rustState.micAvailable,
    micEnabled: rustState.micEnabled,
    micPeak: rustState.micPeak,
};
```

### Worker Communication
```typescript
// Request-response pattern with Promise mapping
const pendingRequests = new Map<number, {resolve, reject}>();

// Always handle worker errors and cleanup
worker.on('error', (err) => {
  rejectAllPending(err);
  worker?.terminate();
});
```

### React State Management
```typescript
// Use refs for audio data to avoid re-renders
const waveformRef = useRef<number[]>(null);

// Separate audio state from UI state for performance
const [audioState, setAudioState] = useState<AudioEngineState>();
const audioStateRef = useRef<AudioEngineState>(); // For stable access
```

### Type Safety
- Strict TypeScript configuration
- Shared types between main/renderer via `src/types.ts`
- IPC type safety through `src/types/electron-api.d.ts`
- MCP tool schemas with proper type definitions

## Build & Development

```bash
# Development with hot reload (from root)
npm start

# Lint check (from root)
npm run lint

# Package build (from app/ directory)
cd app && npm run package

# Create installer
cd app && npm run make
```

## Testing Considerations
- Worker thread communication testing
- Audio timing and synchronization validation
- Memory leak prevention in long-running audio processes
- Error handling for malformed MP3 files
- MCP server endpoint testing
- EQ filter response validation

## Performance Notes
- Audio processing runs on dedicated threads (non-blocking UI)
- Waveform data cached in React refs (no re-render overhead)
- Transferable objects used for large audio buffer transfers
- BPM detection optimized with multi-peak correlation algorithm
- EQ processing uses efficient biquad filters
- MCP controller caches audio state for sub-millisecond responses

---

## Code Style & Conventions
- Use TypeScript strict mode
- Prefer async/await over Promises
- Use EventEmitter for worker communication
- Handle all worker lifecycle events (error, exit, message)
- Implement proper cleanup for audio resources
- After modifying code, run ESLint to catch issues before commit:
  - `npm run lint` (ensure no unexpected warnings/errors; fix or suppress intentionally)