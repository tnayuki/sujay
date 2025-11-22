/* Audio Worker (Pattern A) */
import { parentPort, Worker as NodeWorker } from 'node:worker_threads';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { AudioEngine } from './audio-engine.js';
import type { WorkerInMsg, WorkerOutMsg } from './audio-worker-types.js';
import type { Track } from '../types.js';

if (!parentPort) {
  throw new Error('audio-worker must be started as a Worker');
}

const __filename = fileURLToPath(import.meta.url);
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
    decoderWorker?.terminate().catch(() => {});
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
        parentPort!.postMessage({ type: 'pong', id: msg.id } as WorkerOutMsg);
        break;
      case 'probeDevices': {
        try {
          const portAudio = (await import('naudiodon2')).default as any;
          const devices = portAudio.getDevices ? portAudio.getDevices() : [];
          parentPort!.postMessage({ type: 'probeResult', id: msg.id, ok: true, count: devices.length } as WorkerOutMsg);
        } catch (e: any) {
          parentPort!.postMessage({ type: 'probeResult', id: msg.id, ok: false, error: String(e?.message || e) } as WorkerOutMsg);
        }
        break;
      }
      case 'getDevices': {
        try {
          const portAudio = (await import('naudiodon2')).default as any;
          const raw = portAudio.getDevices ? portAudio.getDevices() : [];
          const devices = raw
            .filter((d: any) => (d.maxOutputChannels ?? 0) > 0)
            .map((d: any) => ({ id: d.id, name: d.name, maxOutputChannels: d.maxOutputChannels }));
          parentPort!.postMessage({ type: 'devices', id: msg.id, devices } as WorkerOutMsg);
        } catch (e: any) {
          parentPort!.postMessage({ type: 'devices', id: msg.id, devices: [] } as WorkerOutMsg);
        }
        break;
      }
      case 'init': {
        try {
          if (!audioEngine) {
            audioEngine = new AudioEngine(decodeTrack);
            // Forward events to main
            audioEngine.on('state-changed', (state) => {
              parentPort!.postMessage({ type: 'stateChanged', state } as WorkerOutMsg);
            });
            audioEngine.on('level-state', (state) => {
              parentPort!.postMessage({ type: 'levelState', state } as WorkerOutMsg);
            });
            audioEngine.on('track-ended', () => {
              parentPort!.postMessage({ type: 'trackEnded' } as WorkerOutMsg);
            });
            audioEngine.on('error', (error) => {
              parentPort!.postMessage({ type: 'error', error: String(error?.message || error) } as WorkerOutMsg);
            });
            audioEngine.on('waveform-chunk', (data) => {
              parentPort!.postMessage({ type: 'waveformChunk', ...data } as WorkerOutMsg);
            });
            audioEngine.on('waveform-complete', (data) => {
              parentPort!.postMessage({ type: 'waveformComplete', ...data } as WorkerOutMsg);
            });
          }
          audioEngine.applyAudioConfig(msg.audioConfig);
          audioEngine.updateOSCConfig(msg.oscConfig);
          await audioEngine.initialize();
          parentPort!.postMessage({ type: 'initResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (e: any) {
          parentPort!.postMessage({ type: 'initResult', id: msg.id, ok: false, error: String(e?.message || e) } as WorkerOutMsg);
        }
        break;
      }
      case 'play': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');
          await audioEngine.play(msg.track, msg.crossfade, msg.targetDeck);
          parentPort!.postMessage({ type: 'playResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (e: any) {
          parentPort!.postMessage({ type: 'playResult', id: msg.id, ok: false, error: String(e?.message || e) } as WorkerOutMsg);
        }
        break;
      }
      case 'stop': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'stopResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.stop(msg.deck);
          parentPort!.postMessage({ type: 'stopResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'seek': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'seekResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.seek(msg.deck, msg.position);
          parentPort!.postMessage({ type: 'seekResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'setCrossfader': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'setCrossfaderResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.setCrossfaderPosition(msg.position);
          parentPort!.postMessage({ type: 'setCrossfaderResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'setMasterTempo': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'setMasterTempoResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.setMasterTempo(msg.bpm);
          parentPort!.postMessage({ type: 'setMasterTempoResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'startDeck': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'startDeckResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.startDeck(msg.deck);
          parentPort!.postMessage({ type: 'startDeckResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'getState': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'stateResult', id: msg.id, state: {} as any } as WorkerOutMsg);
        } else {
          const state = audioEngine.getState();
          parentPort!.postMessage({ type: 'stateResult', id: msg.id, state } as WorkerOutMsg);
        }
        break;
      }
      case 'updateOSCConfig': {
        if (!audioEngine) {
          parentPort!.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: false } as WorkerOutMsg);
        } else {
          audioEngine.updateOSCConfig(msg.config);
          parentPort!.postMessage({ type: 'updateOSCConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
        }
        break;
      }
      case 'applyAudioConfig': {
        try {
          if (!audioEngine) throw new Error('AudioEngine not initialized');
          audioEngine.applyAudioConfig(msg.config);
          await audioEngine.cleanup();
          await audioEngine.initialize();
          parentPort!.postMessage({ type: 'applyAudioConfigResult', id: msg.id, ok: true } as WorkerOutMsg);
        } catch (e: any) {
          parentPort!.postMessage({ type: 'applyAudioConfigResult', id: msg.id, ok: false, error: String(e?.message || e) } as WorkerOutMsg);
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
          decoderWorker.terminate().catch(() => {});
          decoderWorker = null;
        }
        parentPort!.postMessage({ type: 'cleanupResult', id: msg.id, ok: true } as WorkerOutMsg);
        break;
      }
      default:
        // ignore unknown for now
        break;
    }
  } catch (e) {
    console.error('[AudioWorker] Unhandled error:', e);
  }
});

// Ready signal
parentPort.postMessage({ type: 'pong', id: 0 } as WorkerOutMsg);
