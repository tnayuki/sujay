/* Audio Worker (Pattern A) */
import { parentPort, Worker as NodeWorker } from 'node:worker_threads';
import path from 'node:path';
// fileURLToPath import removed; not needed under CJS build
import { AudioEngine } from './audio-engine';
import type { WorkerInMsg, WorkerOutMsg } from './audio-worker-types';
import type { Track } from '../types';

if (!parentPort) {
  throw new Error('audio-worker must be started as a Worker');
}
const port = parentPort; // parentPort is now guaranteed

// __dirname available in CJS context; use it directly
const __dirname = path.dirname(__filename);

let audioEngine: AudioEngine | null = null;
let recordingBridge: RecordingBridge | null = null;

const TARGET_SAMPLE_RATE = 44100;
const TARGET_CHANNELS = 2;
const recordingWriterPath = path.join(__dirname, 'recording-writer.js');

let decoderWorker: NodeWorker | null = null;
let decodeRequestId = 1;
const pendingDecodes = new Map<number, {
  resolve: (value: { pcmData: Float32Array; float32Mono: Float32Array; bpm: number | undefined }) => void;
  reject: (error: Error) => void;
}>();

type DecoderWorkerSuccessMsg = {
  type: 'decoded';
  id: number;
  trackId: string;
  pcm: ArrayBuffer;
  mono: ArrayBuffer;
  bpm: number | undefined;
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
    if (this.worker) {
      return;
    }
    this.worker = new NodeWorker(this.workerPath);
    this.worker.on('message', (msg: RecordingWriterOutMsg) => this.handleWorkerMessage(msg));
    this.worker.on('error', (err) => this.handleWorkerError(err instanceof Error ? err : new Error(String(err))));
    this.worker.on('exit', (code) => {
      if (code !== 0) {
        this.handleWorkerError(new Error(`recording-writer exited with code ${code}`));
      }
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
    if (this.pendingStart) {
      this.pendingStart.reject(error);
      this.pendingStart = null;
    }
    if (this.pendingStop) {
      this.pendingStop.reject(error);
      this.pendingStop = null;
    }
    this.active = false;
    this.reportError(error);
  }

  async start(path: string): Promise<void> {
    if (this.pendingStart) {
      throw new Error('Recording start already pending');
    }
    if (this.active) {
      throw new Error('Recording already active');
    }
    this.ensureWorker();
    if (!this.worker) {
      throw new Error('Recording writer unavailable');
    }
    await new Promise<void>((resolve, reject) => {
      this.pendingStart = { resolve, reject };
      const message: RecordingWriterInMsg = {
        type: 'start',
        path,
        sampleRate: this.sampleRate,
        channels: this.channels,
      };
      this.worker?.postMessage(message);
    });
  }

  handleAudioChunk(buffer: Float32Array, frames: number): void {
    if (!this.active || !this.worker) {
      return;
    }
    const samples = frames * this.channels;
    const copy = buffer.slice(0, samples);
    try {
      this.worker.postMessage({ type: 'write', chunk: copy.buffer } as RecordingWriterInMsg, [copy.buffer]);
    } catch (error) {
      const err = error instanceof Error ? error : new Error(String(error));
      this.handleWorkerError(err);
    }
  }

  async stop(): Promise<number> {
    if (!this.worker) {
      return 0;
    }
    if (!this.active && !this.pendingStart) {
      return 0;
    }
    if (this.pendingStop) {
      throw new Error('Recording stop already pending');
    }
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
    this.pendingStart = null;
    this.pendingStop = null;
  }
}

function ensureDecoderWorker(): void {
  if (decoderWorker) {
    return;
  }

  const workerPath = path.join(__dirname, 'audio-decode-worker.js');
  decoderWorker = new NodeWorker(workerPath);

  decoderWorker.on('message', (msg: DecoderWorkerOutMsg) => {
    if (msg.type === 'decoded') {
      const pending = pendingDecodes.get(msg.id);
      if (!pending) {
        return;
      }
      pendingDecodes.delete(msg.id);
      try {
        const pcmData = new Float32Array(msg.pcm);
        const float32Mono = new Float32Array(msg.mono);
        pending.resolve({ pcmData, float32Mono, bpm: msg.bpm });
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
    decoderWorker?.terminate().catch(() => {
      // Termination failed, ignore
    });
    decoderWorker = null;
  });

  decoderWorker.on('exit', (code) => {
    if (code !== 0) {
      console.warn(`[decode-worker] exited with code ${code}`);
    }
    rejectAllDecodes(new Error('decoder worker exited'));
    decoderWorker = null;
  });
}

function rejectAllDecodes(error: Error): void {
  for (const pending of pendingDecodes.values()) {
    pending.reject(error);
  }
  pendingDecodes.clear();
}

function decodeTrack(track: Track): Promise<{ pcmData: Float32Array; float32Mono: Float32Array; bpm: number | undefined }> {
  if (!track.mp3Path) {
    return Promise.reject(new Error('Track mp3Path missing'));
  }

  ensureDecoderWorker();

  return new Promise((resolve, reject) => {
    if (!decoderWorker) {
      reject(new Error('Decoder worker unavailable'));
      return;
    }

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

parentPort.on('message', async (msg: WorkerInMsg) => {
  try {
    switch (msg.type) {
      case 'ping':
        port.postMessage({ type: 'pong', id: msg.id } as WorkerOutMsg);
        break;
        case 'probeDevices': {
          try {
            const mod = await import('naudiodon2');
            const portAudio = mod.default;
            const devices = typeof portAudio.getDevices === 'function' ? portAudio.getDevices() : [];
          port.postMessage({ type: 'probeResult', id: msg.id, ok: true, count: devices.length } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'probeResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }
      case 'getDevices': {
        try {
            const mod = await import('naudiodon2');
            const portAudio = mod.default;
            const raw = typeof portAudio.getDevices === 'function' ? portAudio.getDevices() : [];
            const devices = raw
              .filter((d: { maxOutputChannels?: number }) => (d.maxOutputChannels ?? 0) > 0)
              .map((d: { id: number; name: string; maxOutputChannels: number }) => ({ id: d.id, name: d.name, maxOutputChannels: d.maxOutputChannels }));
          port.postMessage({ type: 'devices', id: msg.id, devices } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'devices', id: msg.id, devices: [] } as WorkerOutMsg);
        }
        break;
      }
      case 'init': {
        try {
          if (!audioEngine) {
            audioEngine = new AudioEngine(decodeTrack);
            // Forward events to main
            audioEngine.on('state-changed', (state) => {
              port.postMessage({ type: 'stateChanged', state } as WorkerOutMsg);
            });
            audioEngine.on('level-state', (state) => {
              port.postMessage({ type: 'levelState', state } as WorkerOutMsg);
            });
            audioEngine.on('track-ended', () => {
              port.postMessage({ type: 'trackEnded' } as WorkerOutMsg);
            });
            audioEngine.on('error', (error) => {
              const errMsg = error instanceof Error ? error.message : String(error);
              port.postMessage({ type: 'error', error: errMsg } as WorkerOutMsg);
            });
            audioEngine.on('waveform-chunk', (data) => {
              port.postMessage({ type: 'waveformChunk', ...data } as WorkerOutMsg);
            });
            audioEngine.on('waveform-complete', (data) => {
              port.postMessage({ type: 'waveformComplete', ...data } as WorkerOutMsg);
            });
            recordingBridge = new RecordingBridge(
              recordingWriterPath,
              TARGET_SAMPLE_RATE,
              TARGET_CHANNELS,
              (error) => {
                const message = error instanceof Error ? error.message : String(error);
                port.postMessage({ type: 'recordingError', error: message } as WorkerOutMsg);
              },
            );
            audioEngine.setRecordingTap((buffer, frames) => {
              recordingBridge?.handleAudioChunk(buffer, frames);
            });
          }
          audioEngine.applyAudioConfig(msg.audioConfig);
          audioEngine.updateOSCConfig(msg.oscConfig);
          await audioEngine.initialize();
          port.postMessage({ type: 'initResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'initResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }
      case 'play': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');
          await audioEngine.play(msg.track, msg.crossfade, msg.targetDeck);
          port.postMessage({ type: 'playResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'playResult', id: msg.id, ok: false, error } as WorkerOutMsg);
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
      case 'startDeck': {
        if (!audioEngine) {
          port.postMessage({ type: 'startDeckResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.startDeck(msg.deck);
          port.postMessage({ type: 'startDeckResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'setTalkover': {
        if (!audioEngine) {
          port.postMessage({ type: 'setTalkoverResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.setTalkover(msg.pressed);
          port.postMessage({ type: 'setTalkoverResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'getState': {
        if (!audioEngine) {
          port.postMessage({ type: 'stateResult', id: msg.id, state: {} } as WorkerOutMsg);
          } else {
            const state = audioEngine.getState();
            port.postMessage({ type: 'stateResult', id: msg.id, state } as WorkerOutMsg);
          }
          break;
        }
      case 'updateOSCConfig': {
        if (!audioEngine) {
          port.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.updateOSCConfig(msg.config);
          port.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'applyAudioConfig': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');
          audioEngine.applyAudioConfig(msg.config);
          await audioEngine.cleanup();
          await audioEngine.initialize();
          port.postMessage({ type: 'applyAudioConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          port.postMessage({ type: 'applyAudioConfigResult', id: msg.id, ok: false, error } as WorkerOutMsg);
        }
        break;
      }
      case 'startRecording': {
        try {
          if (!recordingBridge) {
            throw new Error('Recording bridge unavailable');
          }
          await recordingBridge.start(msg.path);
          port.postMessage({ type: 'startRecordingResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          port.postMessage({ type: 'startRecordingResult', id: msg.id, ok: false, error: message } as WorkerOutMsg);
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
          const message = error instanceof Error ? error.message : String(error);
          port.postMessage({ type: 'stopRecordingResult', id: msg.id, ok: false, error: message } as WorkerOutMsg);
        }
        break;
      }
      case 'cleanup': {
        if (audioEngine) {
          await audioEngine.cleanup();
          audioEngine = null;
        }
        if (decoderWorker) {
          rejectAllDecodes(new Error('decoder worker cleaned up'));
          decoderWorker.terminate().catch(() => {
            // Termination failed, ignore
          });
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
        // ignore unknown for now
        break;
    }
  } catch (error) {
    console.error('[AudioWorker] Unhandled error:', error);
  }
});

// Ready signal
parentPort.postMessage({ type: 'pong', id: 0 } as WorkerOutMsg);
