/* Audio Worker using Rust AudioEngine */
import { parentPort, Worker as NodeWorker } from 'node:worker_threads';
import path from 'node:path';
import type { WorkerInMsg, WorkerOutMsg } from './audio-worker-types';
import type { Track, TrackStructure, AudioEngineState } from '../types';

if (!parentPort) {
  throw new Error('audio-worker must be started as a Worker');
}
const port = parentPort;

const __dirname = path.dirname(__filename);

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
  setDeckCueEnabled(deck: number, enabled: boolean): void;
  setChannelConfig(mainLeft: number, mainRight: number, cueLeft: number, cueRight: number): void;
  configureDevice(config: DeviceConfig): void;
  getState(): RustAudioEngineStateUpdate;
  close(): void;
}

let audioEngine: RustAudioEngine | null = null;
let recordingBridge: RecordingBridge | null = null;

// Track metadata storage (since Rust engine only stores PCM)
let deckATrack: Track | null = null;
let deckBTrack: Track | null = null;

const TARGET_SAMPLE_RATE = 44100;
const TARGET_CHANNELS = 2;
const recordingWriterPath = path.join(__dirname, 'recording-writer.js');

let decoderWorker: NodeWorker | null = null;
let decodeRequestId = 1;
const pendingDecodes = new Map<number, {
  resolve: (value: { pcmData: Float32Array; float32Mono: Float32Array; bpm: number | undefined; structure: TrackStructure | undefined }) => void;
  reject: (error: Error) => void;
}>();

type DecoderWorkerSuccessMsg = {
  type: 'decoded';
  id: number;
  trackId: string;
  pcm: ArrayBuffer;
  mono: ArrayBuffer;
  bpm: number | undefined;
  structure: TrackStructure | undefined;
  sampleRate: number;
  channels: number;
};

type DecoderWorkerErrorMsg = {
  type: 'decodeError';
  id: number;
  trackId: string;
  error: string;
};

type DecoderWorkerOutMsg = DecoderWorkerSuccessMsg | DecoderWorkerErrorMsg;

type RecordingWriterInMsg =
  | { type: 'start'; path: string; sampleRate: number; channels: number }
  | { type: 'write'; chunk: ArrayBuffer }
  | { type: 'stop' }
  | { type: 'terminate' };

type RecordingWriterOutMsg =
  | { type: 'started'; ok: boolean; error?: string }
  | { type: 'stopped'; ok: boolean; bytesWritten?: number; error?: string }
  | { type: 'error'; error: string };

type PendingPromise<T> = { resolve: (value: T) => void; reject: (error: Error) => void };

class RecordingBridge {
  private worker: NodeWorker | null = null;
  private pendingStart: PendingPromise<void> | null = null;
  private pendingStop: PendingPromise<number> | null = null;
  private active = false;

  constructor(
    private readonly workerPath: string,
    private readonly sampleRate: number,
    private readonly channels: number,
    private readonly reportError: (error: Error) => void,
  ) {}

  private ensureWorker(): void {
    if (this.worker) return;
    this.worker = new NodeWorker(this.workerPath);
    this.worker.on('message', (msg: RecordingWriterOutMsg) => this.handleWorkerMessage(msg));
    this.worker.on('error', (err) => this.handleWorkerError(err instanceof Error ? err : new Error(String(err))));
    this.worker.on('exit', (code) => {
      if (code !== 0) this.handleWorkerError(new Error(`recording-writer exited with code ${code}`));
      this.worker = null;
      this.active = false;
    });
  }

  private handleWorkerMessage(msg: RecordingWriterOutMsg): void {
    if (msg.type === 'started') {
      if (msg.ok) {
        this.active = true;
        this.pendingStart?.resolve();
      } else {
        const error = new Error(msg.error || 'Failed to start recording');
        this.pendingStart?.reject(error);
        this.reportError(error);
      }
      this.pendingStart = null;
    } else if (msg.type === 'stopped') {
      const bytes = msg.bytesWritten ?? 0;
      if (msg.ok) {
        this.pendingStop?.resolve(bytes);
      } else {
        const error = new Error(msg.error || 'Failed to stop recording');
        this.pendingStop?.reject(error);
        this.reportError(error);
      }
      this.pendingStop = null;
      this.active = false;
    } else if (msg.type === 'error') {
      this.handleWorkerError(new Error(msg.error));
    }
  }

  private handleWorkerError(error: Error): void {
    if (this.pendingStart) { this.pendingStart.reject(error); this.pendingStart = null; }
    if (this.pendingStop) { this.pendingStop.reject(error); this.pendingStop = null; }
    this.active = false;
    this.reportError(error);
  }

  async start(filePath: string): Promise<void> {
    if (this.pendingStart) throw new Error('Recording start already pending');
    if (this.active) throw new Error('Recording already active');
    this.ensureWorker();
    if (!this.worker) throw new Error('Recording writer unavailable');
    await new Promise<void>((resolve, reject) => {
      this.pendingStart = { resolve, reject };
      this.worker?.postMessage({ type: 'start', path: filePath, sampleRate: this.sampleRate, channels: this.channels } as RecordingWriterInMsg);
    });
  }

  handleAudioChunk(buffer: Float32Array, frames: number): void {
    if (!this.active || !this.worker) return;
    const samples = frames * this.channels;
    const copy = buffer.slice(0, samples);
    try {
      this.worker.postMessage({ type: 'write', chunk: copy.buffer } as RecordingWriterInMsg, [copy.buffer]);
    } catch (error) {
      this.handleWorkerError(error instanceof Error ? error : new Error(String(error)));
    }
  }

  async stop(): Promise<number> {
    if (!this.worker) return 0;
    if (!this.active && !this.pendingStart) return 0;
    if (this.pendingStop) throw new Error('Recording stop already pending');
    return new Promise<number>((resolve, reject) => {
      this.pendingStop = { resolve, reject };
      this.worker?.postMessage({ type: 'stop' } as RecordingWriterInMsg);
    });
  }

  async dispose(): Promise<void> {
    if (this.worker) {
      this.worker.postMessage({ type: 'terminate' } as RecordingWriterInMsg);
      this.worker = null;
    }
    this.active = false;
  }
}

function ensureDecoderWorker(): void {
  if (decoderWorker) return;

  const workerPath = path.join(__dirname, 'audio-decode-worker.js');
  decoderWorker = new NodeWorker(workerPath);

  decoderWorker.on('message', (msg: DecoderWorkerOutMsg) => {
    if (msg.type === 'decoded') {
      const pending = pendingDecodes.get(msg.id);
      if (!pending) return;
      pendingDecodes.delete(msg.id);
      try {
        const pcmData = new Float32Array(msg.pcm);
        const float32Mono = new Float32Array(msg.mono);
        pending.resolve({ pcmData, float32Mono, bpm: msg.bpm, structure: msg.structure });
      } catch (error) {
        pending.reject(error instanceof Error ? error : new Error(String(error)));
      }
    } else if (msg.type === 'decodeError') {
      const pending = pendingDecodes.get(msg.id);
      if (pending) {
        pendingDecodes.delete(msg.id);
        pending.reject(new Error(msg.error));
      }
    }
  });

  decoderWorker.on('error', (err) => {
    console.error('[decode-worker] error', err);
    rejectAllDecodes(err instanceof Error ? err : new Error(String(err)));
    decoderWorker?.terminate().catch((e) => console.warn('[decode-worker] terminate error', e));
    decoderWorker = null;
  });

  decoderWorker.on('exit', (code) => {
    if (code !== 0) console.warn(`[decode-worker] exited with code ${code}`);
    rejectAllDecodes(new Error('decoder worker exited'));
    decoderWorker = null;
  });
}

function rejectAllDecodes(error: Error): void {
  for (const pending of pendingDecodes.values()) pending.reject(error);
  pendingDecodes.clear();
}

function decodeTrack(track: Track): Promise<{ pcmData: Float32Array; float32Mono: Float32Array; bpm: number | undefined; structure: TrackStructure | undefined }> {
  if (!track.mp3Path) return Promise.reject(new Error('Track mp3Path missing'));
  ensureDecoderWorker();
  return new Promise((resolve, reject) => {
    if (!decoderWorker) { reject(new Error('Decoder worker unavailable')); return; }
    const id = decodeRequestId++;
    pendingDecodes.set(id, { resolve, reject });
    decoderWorker.postMessage({
      type: 'decode',
      id,
      trackId: track.id,
      mp3Path: track.mp3Path,
      sampleRate: TARGET_SAMPLE_RATE,
      channels: TARGET_CHANNELS,
    });
  });
}

// Convert Rust state to TS AudioEngineState format
function convertRustState(rustState: RustAudioEngineStateUpdate): AudioEngineState {
  // Strip large data from tracks to avoid sending huge payloads every frame
  const stripTrackData = (track: Track | null): Track | undefined => {
    if (!track) return undefined;
    return { ...track, pcmData: undefined, waveformData: undefined };
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
    // EQ, mic, talkover - not yet implemented in Rust
    deckAEqCut: { low: false, mid: false, high: false },
    deckBEqCut: { low: false, mid: false, high: false },
    deckAGain: rustState.deckAGain,
    deckBGain: rustState.deckBGain,
    deckACueEnabled: rustState.deckACueEnabled,
    deckBCueEnabled: rustState.deckBCueEnabled,
    isSeek: rustState.updateReason === 'seek',
    micAvailable: false,
    micEnabled: false,
    micWarning: null,
    talkoverActive: false,
    talkoverButtonPressed: false,
    micLevel: 0,
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

            recordingBridge = new RecordingBridge(
              recordingWriterPath,
              TARGET_SAMPLE_RATE,
              TARGET_CHANNELS,
              (error) => {
                port.postMessage({ type: 'recordingError', error: error.message } as WorkerOutMsg);
              }
            );
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

          // Decode if needed
          if (!pcmData) {
            const decoded = await decodeTrack(track);
            pcmData = decoded.pcmData;
            bpm = decoded.bpm;
            waveformData = decoded.float32Mono;
          }

          const targetDeck = msg.deck;

          // Load track to Rust engine
          if (!pcmData) throw new Error('PCM data is required');
          audioEngine.loadTrack(targetDeck, pcmData, bpm, track.id);

          // Store track metadata with waveform data
          const trackWithData = { ...track, pcmData, bpm, waveformData };
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

          // Decode if needed
          if (!pcmData) {
            const decoded = await decodeTrack(track);
            pcmData = decoded.pcmData;
            bpm = decoded.bpm;
            waveformData = decoded.float32Mono;
          }

          // Determine target deck
          const targetDeck = msg.targetDeck ?? (deckATrack ? 2 : 1);

          // Load track to Rust engine
          if (!pcmData) throw new Error('PCM data is required');
          audioEngine.loadTrack(targetDeck, pcmData, bpm, track.id);

          // Store track metadata with waveform data
          const trackWithData = { ...track, pcmData, bpm, waveformData };
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
        // TODO: Implement in Rust
        port.postMessage({ type: 'setEqCutResult', id: msg.id, ok: true } as WorkerOutMsg);
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

      case 'setTalkover': {
        // TODO: Implement in Rust
        port.postMessage({ type: 'setTalkoverResult', id: msg.id, ok: true } as WorkerOutMsg);
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
        // TODO: Implement OSC in TypeScript side
        port.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
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
          if (!recordingBridge) throw new Error('Recording bridge unavailable');
          await recordingBridge.start(msg.path);
          port.postMessage({ type: 'startRecordingResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'startRecordingResult', id: msg.id, ok: false, error: error instanceof Error ? error.message : String(error) } as WorkerOutMsg);
        }
        break;
      }

      case 'stopRecording': {
        try {
          if (!recordingBridge) {
            port.postMessage({ type: 'stopRecordingResult', id: msg.id, ok: true, bytesWritten: 0 } as WorkerOutMsg);
            break;
          }
          const bytes = await recordingBridge.stop();
          port.postMessage({ type: 'stopRecordingResult', id: msg.id, ok: true, bytesWritten: bytes } as WorkerOutMsg);
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
