import { parentPort } from 'node:worker_threads';
import fs from 'node:fs';
import { MPEGDecoder } from 'mpg123-decoder';
import { BPMDetector } from './bpm-detector.js';

interface DecodeRequest {
  type: 'decode';
  id: number;
  trackId: string;
  mp3Path: string;
  sampleRate: number;
  channels: number;
}

interface DecodeSuccess {
  type: 'decoded';
  id: number;
  trackId: string;
  pcm: ArrayBuffer;
  mono: ArrayBuffer;
  bpm: number | undefined;
  sampleRate: number;
  channels: number;
}

interface DecodeError {
  type: 'decodeError';
  id: number;
  trackId: string;
  error: string;
}

type DecodeWorkerOutMsg = DecodeSuccess | DecodeError;

type DecoderMessage = DecodeRequest;

if (!parentPort) {
  throw new Error('audio-decode-worker must be started as a Worker');
}

let decoder: MPEGDecoder | null = null;
let decoderReady: Promise<void> | null = null;

async function ensureDecoder(): Promise<MPEGDecoder> {
  if (!decoder) {
    decoder = new MPEGDecoder();
    decoderReady = decoder.ready;
  }
  if (decoderReady) {
    await decoderReady;
  }
  return decoder!;
}

async function handleDecode(msg: DecodeRequest): Promise<void> {
  const { id, trackId, mp3Path, sampleRate, channels } = msg;
  const start = Date.now();

  try {
    const fileBuffer = await fs.promises.readFile(mp3Path);
    const decoderInstance = await ensureDecoder();
    await decoderInstance.reset();
    const dataView = new Uint8Array(fileBuffer.buffer, fileBuffer.byteOffset, fileBuffer.byteLength);
    const decoded = decoderInstance.decode(dataView);

    if (!decoded.samplesDecoded || decoded.channelData.length === 0) {
      throw new Error('Decoder produced no samples');
    }

    if (decoded.errors.length) {
      console.warn(`[decode-worker] ${decoded.errors.length} decode error(s) reported for track ${trackId}`);
    }

    const sourceChannels = Math.max(1, decoded.channelData.length);
    const resampleNeeded = decoded.sampleRate !== sampleRate;
    const targetFrames = resampleNeeded
      ? Math.max(1, Math.floor(decoded.samplesDecoded * sampleRate / decoded.sampleRate))
      : decoded.samplesDecoded;
    const sampleRateRatio = decoded.sampleRate / sampleRate;

    const pcm = new Float32Array(targetFrames * channels);
    const mono = new Float32Array(targetFrames);

    for (let frame = 0; frame < targetFrames; frame++) {
      const srcIndex = resampleNeeded
        ? Math.min(Math.floor(frame * sampleRateRatio), decoded.samplesDecoded - 1)
        : frame;
      let monoAccum = 0;

      for (let ch = 0; ch < channels; ch++) {
        const srcChannelIndex = Math.min(ch, sourceChannels - 1);
        const channelData = decoded.channelData[srcChannelIndex];
        const sample = channelData ? channelData[srcIndex] : 0;
        monoAccum += sample;
        const clamped = Math.max(-1, Math.min(1, sample));
        pcm[frame * channels + ch] = clamped;
      }

      mono[frame] = monoAccum / channels;
    }

    const durationMs = Date.now() - start;
    console.log(`[decode-worker] Decoded ${trackId} in ${durationMs}ms (${decoded.sampleRate}Hz -> ${sampleRate}Hz)`);

    // Detect BPM from mono data
    console.log(`[decode-worker] Detecting BPM for track ${trackId}`);
    const bpm = BPMDetector.detect(mono, sampleRate) ?? undefined;
    if (bpm) {
      console.log(`[decode-worker] Detected BPM: ${bpm} for track ${trackId}`);
    } else {
      console.log(`[decode-worker] BPM detection failed for track ${trackId}`);
    }

    const pcmBuffer = pcm.buffer;
    const monoBuffer = mono.buffer;
    const transferable: DecodeWorkerOutMsg = {
      type: 'decoded',
      id,
      trackId,
      pcm: pcmBuffer,
      mono: monoBuffer,
      bpm,
      sampleRate,
      channels,
    };

    parentPort!.postMessage(transferable, [pcmBuffer, monoBuffer]);
  } catch (error) {
    const payload: DecodeWorkerOutMsg = {
      type: 'decodeError',
      id,
      trackId,
      error: error instanceof Error ? error.message : String(error),
    };
    parentPort!.postMessage(payload);
  }
}

parentPort.on('message', (msg: DecoderMessage) => {
  if (msg.type === 'decode') {
    handleDecode(msg).catch((err) => {
      const payload: DecodeError = {
        type: 'decodeError',
        id: msg.id,
        trackId: msg.trackId,
        error: err instanceof Error ? err.message : String(err),
      };
      parentPort!.postMessage(payload);
    });
  }
});
