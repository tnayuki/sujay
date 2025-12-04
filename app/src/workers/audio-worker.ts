/* Audio Worker using Rust AudioEngine */
import { parentPort } from 'node:worker_threads';
import type { WorkerInMsg, WorkerOutMsg } from './audio-worker-types';
import type { Track, TrackStructure, AudioEngineState } from '../types';
import { OSCManager } from './osc-manager';

if (!parentPort) {
  throw new Error('audio-worker must be started as a Worker');
}
const port = parentPort;

// Rust AudioEngine types
interface RustAudioEngineStateUpdate {
  deckAPosition?: number;
  deckBPosition?: number;
  deckAPlaying: boolean;
  deckBPlaying: boolean;
  crossfaderPosition: number;
  isCrossfading: boolean;
  deckAPeak: number;
  deckBPeak: number;
  deckAPeakHold: number;
  deckBPeakHold: number;
  masterTempo: number;
  deckATrackId?: string;
  deckBTrackId?: string;
  deckAGain: number;
  deckBGain: number;
  deckACueEnabled: boolean;
  deckBCueEnabled: boolean;
  deckAEqCut: { low: boolean; mid: boolean; high: boolean };
  deckBEqCut: { low: boolean; mid: boolean; high: boolean };
  micAvailable: boolean;
  micEnabled: boolean;
  micPeak: number;
  updateReason: string; // "periodic", "seek", "play", "stop", "load"
}

interface DeviceConfig {
  deviceId?: string;
  mainChannels: number[];
  cueChannels: number[];
}

interface RustAudioEngine {
  loadTrack(deck: number, pcmData: Float32Array, bpm?: number, trackId?: string): void;
  play(deck: number): void;
  stop(deck: number): void;
  seek(deck: number, position: number): void;
  setCrossfaderPosition(position: number): void;
  startCrossfade(targetPosition: number | null, duration: number): void;
  setMasterTempo(bpm: number): void;
  setDeckGain(deck: number, gain: number): void;
  setEqCut(deck: number, band: string, enabled: boolean): void;
  setDeckCueEnabled(deck: number, enabled: boolean): void;
  setChannelConfig(mainLeft: number, mainRight: number, cueLeft: number, cueRight: number): void;
  configureDevice(config: DeviceConfig): void;
  setMicEnabled(enabled: boolean): void;
  setMicGain(gain: number): void;
  setTalkoverDucking(ducking: number): void;
  startRecording(path: string, format: string): void;
  stopRecording(): void;
  getState(): RustAudioEngineStateUpdate;
  close(): void;
}

// Rust decode result type
interface RustDecodeResult {
  pcm: Buffer;
  mono: Buffer;
  bpm: number | null;
  structure: {
    bpm: number;
    intro: { start: number; end: number; beats: number };
    main: { start: number; end: number; beats: number };
    outro: { start: number; end: number; beats: number };
    hotCues: number[];
  } | null;
  sampleRate: number;
  channels: number;
}

let audioEngine: RustAudioEngine | null = null;

// Track metadata storage (since Rust engine only stores PCM)
let deckATrack: Track | null = null;
let deckBTrack: Track | null = null;

// OSC Manager for external broadcasting
let oscManager: OSCManager | null = null;
let lastOSCTempo: number | null = null;
let lastOSCDeckATrackId: string | null = null;
let lastOSCDeckBTrackId: string | null = null;

const TARGET_SAMPLE_RATE = 44100;
const TARGET_CHANNELS = 2;

// Rust decoder function reference
let decodeAudio: ((mp3Path: string, sampleRate: number, channels: number) => RustDecodeResult) | null = null;

function decodeTrack(track: Track): { pcmData: Float32Array; float32Mono: Float32Array; bpm: number | undefined; structure: TrackStructure | undefined } {
  if (!track.mp3Path) throw new Error('Track mp3Path missing');
  if (!decodeAudio) throw new Error('Decoder not initialized');

  console.log(`[audio-worker] Decoding track ${track.id} using Rust decoder`);
  const start = Date.now();

  const result = decodeAudio(track.mp3Path, TARGET_SAMPLE_RATE, TARGET_CHANNELS);

  // Convert Buffer to Float32Array
  const pcmData = new Float32Array(result.pcm.buffer, result.pcm.byteOffset, result.pcm.byteLength / 4);
  const float32Mono = new Float32Array(result.mono.buffer, result.mono.byteOffset, result.mono.byteLength / 4);

  // Convert structure format
  let structure: TrackStructure | undefined;
  if (result.structure) {
    structure = {
      bpm: result.structure.bpm,
      beats: result.structure.beats?.map(b => b) || [],
      intro: result.structure.intro,
      main: result.structure.main,
      outro: result.structure.outro,
      hotCues: result.structure.hotCues?.map(h => h) || [],
    };
  }

  const durationMs = Date.now() - start;
  console.log(`[audio-worker] Decoded ${track.id} in ${durationMs}ms, BPM: ${result.bpm ?? 'unknown'}`);

  return {
    pcmData,
    float32Mono,
    bpm: result.bpm ?? undefined,
    structure,
  };
}

// Convert Rust state to TS AudioEngineState format
function broadcastOSCState(rustState: RustAudioEngineStateUpdate): void {
  if (!oscManager) return;

  // Send master tempo (only when changed)
  if (rustState.masterTempo && rustState.masterTempo !== lastOSCTempo) {
    oscManager.sendMasterTempo(rustState.masterTempo);
    lastOSCTempo = rustState.masterTempo;
  }

  // Send deck A track info (only when changed)
  const deckATrackId = deckATrack?.id ?? null;
  if (deckATrackId !== lastOSCDeckATrackId) {
    oscManager.sendCurrentTrack(deckATrack, 'A');
    lastOSCDeckATrackId = deckATrackId;
  }

  // Send deck B track info (only when changed)
  const deckBTrackId = deckBTrack?.id ?? null;
  if (deckBTrackId !== lastOSCDeckBTrackId) {
    oscManager.sendCurrentTrack(deckBTrack, 'B');
    lastOSCDeckBTrackId = deckBTrackId;
  }
}

function convertRustState(rustState: RustAudioEngineStateUpdate): AudioEngineState {
  // Broadcast OSC state
  broadcastOSCState(rustState);

  // Strip large data from tracks to avoid sending huge payloads every frame
  const stripTrackData = (track: Track | null): Track | undefined => {
    if (!track) return undefined;
    return { ...track, pcmData: undefined, waveformData: undefined, structure: undefined };
  };

  return {
    deckA: stripTrackData(deckATrack),
    deckB: stripTrackData(deckBTrack),
    deckAPosition: rustState.deckAPosition,
    deckBPosition: rustState.deckBPosition,
    deckAPlaying: rustState.deckAPlaying,
    deckBPlaying: rustState.deckBPlaying,
    isPlaying: rustState.deckAPlaying || rustState.deckBPlaying,
    isCrossfading: rustState.isCrossfading,
    crossfadeProgress: rustState.crossfaderPosition,
    crossfaderPosition: rustState.crossfaderPosition,
    masterTempo: rustState.masterTempo,
    deckAPeak: rustState.deckAPeak,
    deckBPeak: rustState.deckBPeak,
    deckAPeakHold: rustState.deckAPeakHold,
    deckBPeakHold: rustState.deckBPeakHold,
    deckAEqCut: rustState.deckAEqCut,
    deckBEqCut: rustState.deckBEqCut,
    deckAGain: rustState.deckAGain,
    deckBGain: rustState.deckBGain,
    deckACueEnabled: rustState.deckACueEnabled,
    deckBCueEnabled: rustState.deckBCueEnabled,
    isSeek: rustState.updateReason === 'seek',
    micAvailable: rustState.micAvailable,
    micEnabled: rustState.micEnabled,
    micWarning: null,
    talkoverActive: false, // TODO: Track talkover state in Rust
    talkoverButtonPressed: false, // TODO: Track talkover button in Rust
    micLevel: rustState.micPeak,
  };
}

parentPort.on('message', async (msg: WorkerInMsg) => {
  try {
    switch (msg.type) {
      case 'ping':
        port.postMessage({ type: 'pong', id: msg.id } as WorkerOutMsg);
        break;

      case 'probeDevices': {
        try {
          const mod = await import('@sujay/audio');
          const devices = typeof mod.listAudioDevices === 'function' ? mod.listAudioDevices() : [];
          port.postMessage({ type: 'probeResult', id: msg.id, ok: true, count: devices.length } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'probeResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }

      case 'getDevices': {
        try {
          const mod = await import('@sujay/audio');
          const raw = typeof mod.listAudioDevices === 'function' ? mod.listAudioDevices() : [];
          const devices = raw
            .filter((d: { maxOutputChannels?: number }) => (d.maxOutputChannels ?? 0) > 0)
            .map((d: { name: string; maxOutputChannels: number }) => ({
              name: d.name,
              maxOutputChannels: d.maxOutputChannels,
            }));
          port.postMessage({ type: 'devices', id: msg.id, devices } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'devices', id: msg.id, devices: [] } as WorkerOutMsg);
        }
        break;
      }

      case 'init': {
        try {
          if (!audioEngine) {
            const mod = await import('@sujay/audio');

            // Initialize Rust decoder
            decodeAudio = mod.decodeAudio;

            audioEngine = new mod.AudioEngine(
              null,  // deviceId will be set via configureDevice
              2,     // initial channels (will be updated)
              TARGET_SAMPLE_RATE,
              (rustState: RustAudioEngineStateUpdate) => {
                const state = convertRustState(rustState);
                port.postMessage({ type: 'stateChanged', state } as WorkerOutMsg);
              }
            );

            const deviceId = msg.audioConfig?.deviceId;
            const mainChannels = msg.audioConfig?.mainChannels ?? [0, 1];
            const cueChannels = msg.audioConfig?.cueChannels ?? [null, null];

            audioEngine.configureDevice({
              deviceId,
              mainChannels: mainChannels.map((c) => c ?? -1),
              cueChannels: cueChannels.map((c) => c ?? -1),
            });

            // Initialize OSCManager with config from init message
            if (msg.oscConfig) {
              oscManager = new OSCManager(msg.oscConfig);
            }
          }
          port.postMessage({ type: 'initResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          console.error('[AudioWorker] init error:', error);
          port.postMessage({ type: 'initResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }

      case 'loadTrack': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');

          const track = msg.track;
          let pcmData = track.pcmData;
          let bpm = track.bpm;
          let waveformData = track.waveformData;
          let structure = track.structure;

          // Decode if needed (now synchronous via Rust)
          if (!pcmData) {
            const decoded = decodeTrack(track);
            pcmData = decoded.pcmData;
            bpm = decoded.bpm;
            waveformData = decoded.float32Mono;
            structure = decoded.structure;
          }

          const targetDeck = msg.deck;

          // Load track to Rust engine
          if (!pcmData) throw new Error('PCM data is required');
          audioEngine.loadTrack(targetDeck, pcmData, bpm, track.id);

          // Store track metadata with waveform data and structure
          const trackWithData = { ...track, pcmData, bpm, waveformData, structure };
          if (targetDeck === 1) {
            deckATrack = trackWithData;
          } else {
            deckBTrack = trackWithData;
          }

          // Send waveform data in chunks (IPC can't handle large arrays)
          if (waveformData) {
            const CHUNK_SIZE = 44100; // 1 second of samples
            const totalFrames = waveformData.length;
            const totalChunks = Math.ceil(totalFrames / CHUNK_SIZE);
            
            for (let i = 0; i < totalChunks; i++) {
              const start = i * CHUNK_SIZE;
              const end = Math.min(start + CHUNK_SIZE, totalFrames);
              const chunk = Array.from(waveformData.slice(start, end));
              
              port.postMessage({
                type: 'waveformChunk',
                trackId: track.id,
                chunkIndex: i,
                totalChunks,
                chunk,
              } as WorkerOutMsg);
              
              // Yield to prevent blocking
              await new Promise((resolve) => setImmediate(resolve));
            }
            
            port.postMessage({
              type: 'waveformComplete',
              trackId: track.id,
              totalFrames,
            } as WorkerOutMsg);
          }

          // Send structure separately (once per track load)
          if (structure) {
            port.postMessage({
              type: 'trackStructure',
              trackId: track.id,
              deck: targetDeck,
              structure,
            } as WorkerOutMsg);
          }

          port.postMessage({ type: 'loadTrackResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          console.error('[AudioWorker] loadTrack error:', error);
          port.postMessage({ type: 'loadTrackResult', id: msg.id, ok: false, error: String(error) } as WorkerOutMsg);
        }
        break;
      }

      case 'play': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');

          const track = msg.track;
          let pcmData = track.pcmData;
          let bpm = track.bpm;
          let waveformData = track.waveformData;
          let structure = track.structure;

          // Decode if needed (now synchronous via Rust)
          if (!pcmData) {
            const decoded = decodeTrack(track);
            pcmData = decoded.pcmData;
            bpm = decoded.bpm;
            waveformData = decoded.float32Mono;
            structure = decoded.structure;
          }

          // Determine target deck
          const targetDeck = msg.targetDeck ?? (deckATrack ? 2 : 1);

          // Load track to Rust engine
          if (!pcmData) throw new Error('PCM data is required');
          audioEngine.loadTrack(targetDeck, pcmData, bpm, track.id);

          // Store track metadata with waveform data and structure
          const trackWithData = { ...track, pcmData, bpm, waveformData, structure };
          if (targetDeck === 1) {
            deckATrack = trackWithData;
          } else {
            deckBTrack = trackWithData;
          }

          // Handle crossfade
          if (msg.crossfade && (deckATrack || deckBTrack)) {
            const duration = msg.crossfadeDuration ?? 2;
            const targetPosition = msg.crossfadeTargetPosition ?? (targetDeck === 2 ? 1 : 0);
            audioEngine.startCrossfade(targetPosition, duration);
          }

          // Start playback
          audioEngine.play(targetDeck);

          port.postMessage({ type: 'playResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          console.error('[AudioWorker] play error:', error);
          port.postMessage({ type: 'playResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }

      case 'startDeck': {
        if (!audioEngine) {
          port.postMessage({ type: 'startDeckResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.play(msg.deck);
          port.postMessage({ type: 'startDeckResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'stop': {
        if (!audioEngine) {
          port.postMessage({ type: 'stopResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.stop(msg.deck);
          port.postMessage({ type: 'stopResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'seek': {
        if (!audioEngine) {
          port.postMessage({ type: 'seekResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.seek(msg.deck, msg.position);
          port.postMessage({ type: 'seekResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'setCrossfader': {
        if (!audioEngine) {
          port.postMessage({ type: 'setCrossfaderResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.setCrossfaderPosition(msg.position);
          port.postMessage({ type: 'setCrossfaderResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'startCrossfade': {
        if (!audioEngine) {
          port.postMessage({ type: 'startCrossfadeResult', id: msg.id, ok: false, error: 'AudioEngine not initialized' } as WorkerOutMsg);
        } else {
          try {
            audioEngine.startCrossfade(msg.targetPosition ?? null, msg.duration);
            port.postMessage({ type: 'startCrossfadeResult', id: msg.id, ok: true } as WorkerOutMsg);
          } catch (error) {
            port.postMessage({ type: 'startCrossfadeResult', id: msg.id, ok: false, error: error instanceof Error ? error.message : String(error) } as WorkerOutMsg);
          }
        }
        break;
      }

      case 'setMasterTempo': {
        if (!audioEngine) {
          port.postMessage({ type: 'setMasterTempoResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.setMasterTempo(msg.bpm);
          port.postMessage({ type: 'setMasterTempoResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'setDeckCue': {
        if (!audioEngine) {
          port.postMessage({ type: 'setDeckCueResult', id: msg.id, ok: false, error: 'AudioEngine not initialized' } as WorkerOutMsg);
        } else {
          audioEngine.setDeckCueEnabled(msg.deck, msg.enabled);
          port.postMessage({ type: 'setDeckCueResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'setEqCut': {
        if (!audioEngine) {
          port.postMessage({ type: 'setEqCutResult', id: msg.id, ok: false, error: 'AudioEngine not initialized' } as WorkerOutMsg);
        } else {
          audioEngine.setEqCut(msg.deck, msg.band, msg.enabled);
          port.postMessage({ type: 'setEqCutResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'setDeckGain': {
        if (!audioEngine) {
          port.postMessage({ type: 'setDeckGainResult', id: msg.id, ok: false, error: 'AudioEngine not initialized' } as WorkerOutMsg);
        } else {
          audioEngine.setDeckGain(msg.deck, msg.gain);
          port.postMessage({ type: 'setDeckGainResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'setMicEnabled': {
        if (!audioEngine) {
          port.postMessage({ type: 'setMicEnabledResult', id: msg.id, ok: false, error: 'AudioEngine not initialized' } as WorkerOutMsg);
        } else {
          audioEngine.setMicEnabled(msg.enabled);
          port.postMessage({ type: 'setMicEnabledResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }

      case 'getState': {
        if (!audioEngine) {
          port.postMessage({ type: 'stateResult', id: msg.id, state: {} } as WorkerOutMsg);
        } else {
          const rustState = audioEngine.getState();
          const state = convertRustState(rustState);
          port.postMessage({ type: 'stateResult', id: msg.id, state } as WorkerOutMsg);
        }
        break;
      }

      case 'updateOSCConfig': {
        try {
          if (msg.config) {
            if (!oscManager) {
              oscManager = new OSCManager(msg.config);
            } else {
              oscManager.updateConfig(msg.config);
            }
          }
          port.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          console.error('[AudioWorker] updateOSCConfig error:', error);
          port.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: false } as WorkerOutMsg);
        }
        break;
      }

      case 'applyAudioConfig': {
        try {
          if (audioEngine && msg.config) {
            const mainChannels = msg.config.mainChannels ?? [0, 1];
            const cueChannels = msg.config.cueChannels ?? [null, null];
            audioEngine.configureDevice({
              deviceId: msg.config.deviceId,
              mainChannels: mainChannels.map((c) => c ?? -1),
              cueChannels: cueChannels.map((c) => c ?? -1),
            });
          }
          port.postMessage({ type: 'applyAudioConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'applyAudioConfigResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }

      case 'startRecording': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');
          audioEngine.startRecording(msg.path, msg.format);
          port.postMessage({ type: 'startRecordingResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'startRecordingResult', id: msg.id, ok: false, error: error instanceof Error ? error.message : String(error) } as WorkerOutMsg);
        }
        break;
      }

      case 'stopRecording': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');
          audioEngine.stopRecording();
          port.postMessage({ type: 'stopRecordingResult', id: msg.id, ok: true, bytesWritten: 0 } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'stopRecordingResult', id: msg.id, ok: false, error: error instanceof Error ? error.message : String(error) } as WorkerOutMsg);
        }
        break;
      }

      case 'cleanup': {
        if (audioEngine) {
          audioEngine.close();
          audioEngine = null;
        }
        deckATrack = null;
        deckBTrack = null;
        if (decoderWorker) {
          rejectAllDecodes(new Error('decoder worker cleaned up'));
          decoderWorker.terminate().catch((e) => console.warn('[decode-worker] terminate error', e));
          decoderWorker = null;
        }
        if (recordingBridge) {
          await recordingBridge.dispose();
          recordingBridge = null;
        }
        port.postMessage({ type: 'cleanupResult', id: msg.id, ok: true } as WorkerOutMsg);
        break;
      }

      default:
        break;
    }
  } catch (error) {
    console.error('[AudioWorker] Unhandled error:', error);
  }
});

// Ready signal
parentPort.postMessage({ type: 'pong', id: 0 } as WorkerOutMsg);
