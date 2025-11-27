/**
 * Recording writer worker
 * Receives Float32 PCM chunks and writes 44.1kHz/16-bit stereo WAV files.
 */

import { parentPort } from 'node:worker_threads';
import { promises as fs } from 'node:fs';
import type { FileHandle } from 'node:fs/promises';

if (!parentPort) {
  throw new Error('recording-writer must be started as a Worker');
}

const port = parentPort;

interface StartMessage {
  type: 'start';
  path: string;
  sampleRate: number;
  channels: number;
}

interface WriteMessage {
  type: 'write';
  chunk: ArrayBuffer;
}

interface StopMessage {
  type: 'stop';
}

interface TerminateMessage {
  type: 'terminate';
}

type InboundMessage = StartMessage | WriteMessage | StopMessage | TerminateMessage;

type OutboundMessage =
  | { type: 'started'; ok: true }
  | { type: 'started'; ok: false; error: string }
  | { type: 'stopped'; ok: true; bytesWritten: number }
  | { type: 'stopped'; ok: false; error: string }
  | { type: 'error'; error: string };

let fileHandle: FileHandle | null = null;
let bytesWritten = 0;
let currentSampleRate = 44100;
let currentChannels = 2;
let isRecording = false;
const ignoreError = (): void => { /* noop */ };

function createWavHeader(dataSize: number, sampleRate: number, channels: number): Buffer {
  const blockAlign = channels * 2; // 16-bit PCM per channel
  const byteRate = sampleRate * blockAlign;
  const buffer = Buffer.alloc(44);

  buffer.write('RIFF', 0);
  buffer.writeUInt32LE(36 + dataSize, 4);
  buffer.write('WAVE', 8);
  buffer.write('fmt ', 12);
  buffer.writeUInt32LE(16, 16); // Subchunk1Size for PCM
  buffer.writeUInt16LE(1, 20); // AudioFormat (PCM)
  buffer.writeUInt16LE(channels, 22);
  buffer.writeUInt32LE(sampleRate, 24);
  buffer.writeUInt32LE(byteRate, 28);
  buffer.writeUInt16LE(blockAlign, 32);
  buffer.writeUInt16LE(16, 34); // BitsPerSample
  buffer.write('data', 36);
  buffer.writeUInt32LE(dataSize, 40);

  return buffer;
}

async function writeHeader(dataSize: number): Promise<void> {
  if (!fileHandle) {
    throw new Error('File handle not available for header write');
  }
  const header = createWavHeader(dataSize, currentSampleRate, currentChannels);
  await fileHandle.write(header, 0, header.length, 0);
}

async function handleStart(msg: StartMessage): Promise<void> {
  if (isRecording) {
    throw new Error('Recording already in progress');
  }
  await fileHandle?.close().catch(ignoreError);
  fileHandle = await fs.open(msg.path, 'w');
  currentSampleRate = msg.sampleRate;
  currentChannels = msg.channels;
  bytesWritten = 0;
  await writeHeader(0);
  isRecording = true;
}

function floatToPCM16(chunk: ArrayBuffer): Buffer {
  const floatData = new Float32Array(chunk);
  const int16 = new Int16Array(floatData.length);
  for (let i = 0; i < floatData.length; i += 1) {
    const sample = Math.max(-1, Math.min(1, floatData[i]));
    int16[i] = sample < 0 ? Math.round(sample * 0x8000) : Math.round(sample * 0x7fff);
  }
  return Buffer.from(int16.buffer);
}

async function handleWrite(msg: WriteMessage): Promise<void> {
  if (!isRecording || !fileHandle) {
    return;
  }
  const buffer = floatToPCM16(msg.chunk);
  await fileHandle.write(buffer);
  bytesWritten += buffer.length;
}

async function handleStop(): Promise<number> {
  if (!isRecording) {
    return 0;
  }
  await writeHeader(bytesWritten);
  await fileHandle?.close();
  fileHandle = null;
  isRecording = false;
  return bytesWritten;
}

async function cleanup(): Promise<void> {
  if (fileHandle) {
    await fileHandle.close().catch(ignoreError);
    fileHandle = null;
  }
  isRecording = false;
  bytesWritten = 0;
}

port.on('message', async (msg: InboundMessage) => {
  try {
    if (msg.type === 'start') {
      await handleStart(msg);
      port.postMessage({ type: 'started', ok: true } as OutboundMessage);
    } else if (msg.type === 'write') {
      await handleWrite(msg);
    } else if (msg.type === 'stop') {
      const total = await handleStop();
      port.postMessage({ type: 'stopped', ok: true, bytesWritten: total } as OutboundMessage);
    } else if (msg.type === 'terminate') {
      await cleanup();
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (msg.type === 'start') {
      port.postMessage({ type: 'started', ok: false, error: message } as OutboundMessage);
    } else if (msg.type === 'stop') {
      port.postMessage({ type: 'stopped', ok: false, error: message } as OutboundMessage);
    } else {
      port.postMessage({ type: 'error', error: message } as OutboundMessage);
    }
  }
});

port.on('close', () => {
  cleanup().catch(ignoreError);
});
