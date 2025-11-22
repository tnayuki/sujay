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

const TARGET_SAMPLE_RATE = 44100;
const TARGET_CHANNELS = 2;

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
      case 'startDeck': {
        if (!audioEngine) {
          port.postMessage({ type: 'startDeckResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.startDeck(msg.deck);
          port.postMessage({ type: 'startDeckResult', id: msg.id, ok: true } as WorkerOutMsg);
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
