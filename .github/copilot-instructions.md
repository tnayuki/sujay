# Sujay - AI DJ Application

## Project Overview
Sujay is a professional AI-powered DJ application built with Electron, TypeScript, and React. It integrates with Suno AI for music generation and provides a complete DJ experience with dual decks, crossfading, and waveform visualization.

## Current Architecture

### Core Technologies
- **Runtime**: Node.js 22+ with Electron
- **Languages**: TypeScript (strict mode)
- **UI Framework**: React with Vite build system
- **Audio Engine**: Worker-based architecture with:
  - **MP3 Decoding**: `mpg123-decoder` (WASM-based)
  - **Audio Output**: `@sujay/audio` (Rust + cpal via napi-rs)
  - **Time Stretching**: SoundTouch integration for tempo adjustment
  - **BPM Detection**: Custom algorithm with multi-peak correlation
  - **EQ Processing**: 3-band EQ with kill switches (Low/Mid/High)
  - **Recording**: WAV file recording to disk
  - **OSC Broadcasting**: Real-time state broadcasting for external controllers

### Worker Architecture
```
Main Process → Audio Worker → Decode Worker
                ↓                ↓
            Audio Engine    MP3 → PCM + BPM
                ↓
         Recording Writer (optional)
```

**Audio Workers**:
- `src/workers/audio-worker.ts`: Main audio processing loop and engine management
- `src/workers/audio-decode-worker.ts`: MP3 decoding and BPM detection (separate thread)
- `src/workers/audio-engine.ts`: Core playback logic with dual deck support
- `src/workers/bpm-detector.ts`: Multi-peak correlation BPM detection
- `src/workers/time-stretcher.ts`: SoundTouch-based tempo adjustment
- `src/workers/eq-processor.ts`: 3-band biquad filter implementation
- `src/workers/recording-writer.ts`: WAV file writing in separate thread
- `src/workers/osc-manager.ts`: OSC message broadcasting

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

#### ✅ Library Management
- Suno AI integration with workspace support
- Offline-first metadata caching (JSON-based)
- Track structure analysis for optimal DJ mixing
- Right-click context menu for deck loading
- Real-time generation status and download progress

#### ✅ Recording & Broadcasting
- Session recording to WAV files
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
│   │   │   ├── audio-worker.ts
│   │   │   ├── audio-decode-worker.ts
│   │   │   ├── audio-engine.ts
│   │   │   ├── bpm-detector.ts
│   │   │   ├── time-stretcher.ts
│   │   │   ├── eq-processor.ts
│   │   │   ├── recording-writer.ts
│   │   │   └── osc-manager.ts
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
│   └── audio/                 # Native audio I/O (Rust + cpal via napi-rs)
│       ├── src/lib.rs         # cpal bindings for output/input streams
│       ├── Cargo.toml         # Rust dependencies
│       └── index.d.ts         # TypeScript declarations
├── patches/                   # npm package patches
└── package.json               # Workspace root
```

### Key Components

**Audio Engine** (`app/src/workers/audio-engine.ts`):
- Dual deck playback with independent positions
- Crossfade management (auto + manual)
- 3-band EQ processing with kill switches
- Waveform generation and state emission
- Cue monitoring support
- OSC state broadcasting
- Dependency injection pattern for decode functions

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

### Audio Engine Patterns
```typescript
// Always use dependency injection for decode functions
const audioEngine = new AudioEngine(decodeTrack);

// Position updates require explicit emission flags
this.shouldEmitPosition = true;
this.isSeekOperation = true; // For UI seek handling

// Use transferable objects for audio data
parentPort.postMessage(result, [pcmBuffer, monoBuffer]);

// EQ processing per deck
this.deckAEq.process(mixedLeft, mixedRight, deckAEqCut);
this.deckBEq.process(mixedLeft, mixedRight, deckBEqCut);
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