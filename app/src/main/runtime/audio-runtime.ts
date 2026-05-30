/**
 * Audio runtime — Electron-free core for all DJ audio operations.
 *
 * This module owns the Rust AudioEngine lifecycle, deck state, waveform
 * buffering, OSC broadcasting, and recording.  It exposes a plain
 * TypeScript interface that can be driven by any host: Electron IPC
 * handlers today, a future Rust-native host tomorrow.
 */

import { promises as fs } from 'node:fs';
import path from 'node:path';

import { OSCManager } from '../../workers/osc-manager';
import type { AppSettingsStore } from '../settings/app-settings-store';
import type {
  AudioConfig,
  AudioEngineState,
  AudioLevelState,
  EqBand,
  OSCConfig,
  RecordingConfig,
  RecordingFileInfo,
  RecordingStatus,
  Track,
  TrackStructure,
} from '../../types';

// ---------------------------------------------------------------------------
// Rust NAPI types
// ---------------------------------------------------------------------------

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
  deckALoop: { enabled: boolean; start: number; end: number };
  deckBLoop: { enabled: boolean; start: number; end: number };
  sampleRate: number;
  deckATotalFrames?: number;
  deckBTotalFrames?: number;
  micAvailable: boolean;
  micEnabled: boolean;
  micPeak: number;
  updateReason: string;
}

interface RustDecodeResult {
  pcm: Buffer;
  mono: Buffer;
  bpm?: number;
  structure?: {
    bpm: number;
    beats: number[];
    intro: { start: number; end: number; beats: number };
    main: { start: number; end: number; beats: number };
    outro: { start: number; end: number; beats: number };
    hotCues: number[];
  };
  sampleRate: number;
  channels: number;
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
  configureDevice(config: { deviceId?: string; mainChannels?: number[]; cueChannels?: number[] }): void;
  setMicEnabled(enabled: boolean): void;
  setBeatLoop(deck: number, startSeconds: number, endSeconds: number): void;
  clearLoop(deck: number): void;
  startRecording(path: string, format: string): void;
  stopRecording(): void;
  getState(): RustAudioEngineStateUpdate;
  close(): void;
}

interface RustAudioModule {
  AudioEngine: new (
    deviceId?: string | null,
    channels?: number | null,
    sampleRate?: number | null,
    stateCallback?: (state: RustAudioEngineStateUpdate) => void,
  ) => RustAudioEngine;
  decodeAudio: (mp3Path: string, targetSampleRate: number, targetChannels: number) => RustDecodeResult;
  listAudioDevices: () => Array<{ name: string; maxOutputChannels: number }>;
}

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

export const AUDIO_SAMPLE_RATE = 44100;
export const AUDIO_CHANNELS = 2;

// ---------------------------------------------------------------------------
// Callbacks fired by the runtime toward the host
// ---------------------------------------------------------------------------

export interface AudioRuntimeCallbacks {
  /** Fired whenever the audio engine emits a state update. */
  onStateUpdate(state: AudioEngineState, levelState: AudioLevelState): void;
  /** Fired for each waveform chunk streamed during track decode. */
  onWaveformChunk(data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }): void;
  /** Fired when all waveform chunks for a track have been delivered. */
  onWaveformComplete(data: { trackId: string; totalFrames: number }): void;
  /** Fired when a complete flat waveform is assembled for native UI rendering. */
  onNativeWaveformReady(deck: 1 | 2, samples: number[]): void;
  /** Fired when track structure analysis is available for a loaded deck. */
  onTrackStructure(data: { trackId: string; deck: 1 | 2; structure: TrackStructure }): void;
  /** Fired whenever the recording status changes. */
  onRecordingStatus(status: RecordingStatus): void;
}

// ---------------------------------------------------------------------------
// Public AudioRuntime interface
// ---------------------------------------------------------------------------

export interface AudioRuntime {
  // Initialisation (idempotent)
  initialize(): Promise<void>;

  // Deck operations
  loadTrack(track: Track, deck: 1 | 2): Promise<void>;
  play(track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null): Promise<void>;
  stop(deck: 1 | 2): void;
  seek(deck: 1 | 2, position: number): void;
  setCrossfader(position: number): void;
  setMasterTempo(bpm: number): void;
  setDeckCue(deck: 1 | 2, enabled: boolean): void;
  setEqCut(deck: 1 | 2, band: EqBand, enabled: boolean): void;
  setDeckGain(deck: 1 | 2, gain: number): void;
  setBeatLoop(deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]): void;
  clearLoop(deck: 1 | 2): void;
  startDeck(deck: 1 | 2): void;
  setMicEnabled(enabled: boolean): void;

  // Device / config
  listDevices(): Promise<Array<{ name: string; maxOutputChannels: number }>>;
  applyAudioConfig(config: AudioConfig): void;
  updateOscConfig(config: OSCConfig): void;

  // Recording
  getRecordingStatus(): RecordingStatus;
  startRecording(format: 'wav' | 'ogg'): Promise<RecordingStatus>;
  stopRecording(): Promise<RecordingStatus>;

  // State accessors (for native UI / IPC response)
  getCachedState(): AudioEngineState;
  getCachedLevelState(): AudioLevelState;
  getCachedSampleRate(): number;
  getCachedMasterTempo(): number;
  getDeckAPosition(): number;
  getDeckBPosition(): number;
  getDeckATotalFrames(): number;
  getDeckBTotalFrames(): number;
  getDeckATrack(): Track | null;
  getDeckBTrack(): Track | null;
  getDeckALoopBeats(): number | null;
  getDeckBLoopBeats(): number | null;
  getDeckATrackId(): string | null;
  getDeckBTrackId(): string | null;
  getTrackStructure(trackId: string): TrackStructure | undefined;

  // Lifecycle
  close(): void;
}

// ---------------------------------------------------------------------------
// Recording helpers
// ---------------------------------------------------------------------------

const MAX_TIMESTAMP_SUFFIX = 1000;
const padNumber = (value: number, width = 2) => value.toString().padStart(width, '0');

function buildTimestampLabel(date: Date) {
  const year = date.getFullYear();
  const month = padNumber(date.getMonth() + 1);
  const day = padNumber(date.getDate());
  const hours = padNumber(date.getHours());
  const minutes = padNumber(date.getMinutes());
  const seconds = padNumber(date.getSeconds());
  return `${year}${month}${day}-${hours}${minutes}${seconds}`;
}

function recordingExtensionForFormat(format: 'wav' | 'ogg') {
  return format === 'ogg' ? '.ogg' : '.wav';
}

async function pathExists(filePath: string) {
  try {
    await fs.access(filePath);
    return true;
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === 'ENOENT') return false;
    throw err;
  }
}

async function generateTimestampFilePath(directory: string, date: Date, extension: string) {
  const base = buildTimestampLabel(date);
  for (let suffix = 0; suffix < MAX_TIMESTAMP_SUFFIX; suffix += 1) {
    const suffixPart = suffix === 0 ? '' : `-${suffix}`;
    const candidate = path.join(directory, `${base}${suffixPart}${extension}`);
    if (!(await pathExists(candidate))) return candidate;
  }
  throw new Error('Unable to allocate timestamp-based recording filename (too many collisions)');
}

async function generateSequentialFilePath(directory: string, extension: string) {
  for (let index = 1; index < 10000; index += 1) {
    const candidate = path.join(directory, `${padNumber(index, 4)}${extension}`);
    if (!(await pathExists(candidate))) return candidate;
  }
  throw new Error('Unable to allocate recording filename (too many existing recordings)');
}

async function prepareRecordingFile(config: RecordingConfig, format: 'wav' | 'ogg'): Promise<RecordingFileInfo> {
  const createdAt = Date.now();
  const ext = recordingExtensionForFormat(format);
  const filePath = config.namingStrategy === 'timestamp'
    ? await generateTimestampFilePath(config.directory, new Date(createdAt), ext)
    : await generateSequentialFilePath(config.directory, ext);
  return { path: filePath, createdAt, bytesWritten: 0 };
}

async function ensureRecordingDirectory(config: RecordingConfig) {
  if (!path.isAbsolute(config.directory)) {
    throw new Error('Recording directory must be an absolute path');
  }
  try {
    await fs.access(config.directory);
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === 'ENOENT') {
      if (!config.autoCreateDirectory) {
        throw new Error(`Recording directory not found: ${config.directory}`);
      }
      await fs.mkdir(config.directory, { recursive: true });
      return;
    }
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Waveform chunk buffering (for native UI assembly)
// ---------------------------------------------------------------------------

type WaveformChunkBuffer = {
  totalChunks: number;
  compactChunks: Array<number[] | undefined>;
};

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

export function createAudioRuntime(
  settingsStore: AppSettingsStore,
  callbacks: AudioRuntimeCallbacks,
): AudioRuntime {
  // --- Mutable state --------------------------------------------------------
  let audioModule: RustAudioModule | null = null;
  let audioEngine: RustAudioEngine | null = null;
  let decodeAudio: RustAudioModule['decodeAudio'] | null = null;
  let oscManager: OSCManager | null = null;

  let deckATrack: Track | null = null;
  let deckBTrack: Track | null = null;
  let deckALoopBeats: number | null = null;
  let deckBLoopBeats: number | null = null;
  let lastOSCTempo: number | null = null;
  let lastOSCDeckATrackId: string | null = null;
  let lastOSCDeckBTrackId: string | null = null;

  let deckATrackId: string | null = null;
  let deckBTrackId: string | null = null;
  let cachedMasterTempo = 130;
  let cachedDeckAPosition = 0;
  let cachedDeckBPosition = 0;
  let cachedSampleRate = AUDIO_SAMPLE_RATE;

  let recordingStatus: RecordingStatus = { state: 'idle' };

  const trackStructureMap = new Map<string, TrackStructure>();
  const nativeWaveformBuffers = new Map<string, WaveformChunkBuffer>();

  let cachedAudioState: AudioEngineState = {
    deckAPlaying: false,
    deckBPlaying: false,
    isPlaying: false,
    isCrossfading: false,
    crossfadeProgress: 0,
    crossfaderPosition: 0,
    deckAPeak: 0,
    deckBPeak: 0,
    deckAPeakHold: 0,
    deckBPeakHold: 0,
    deckACueEnabled: false,
    deckBCueEnabled: false,
    micAvailable: false,
    micEnabled: false,
    micWarning: null,
    talkoverActive: false,
    talkoverButtonPressed: false,
    micLevel: 0,
  };
  let cachedLevelState: AudioLevelState = {
    deckAPeak: 0,
    deckBPeak: 0,
    deckAPeakHold: 0,
    deckBPeakHold: 0,
    micLevel: 0,
    talkoverActive: false,
    talkoverButtonPressed: false,
  };

  // --- Internal helpers -----------------------------------------------------

  function stripTrackData(track: Track | null): Track | undefined {
    if (!track) return undefined;
    return { ...track, pcmData: undefined, waveformData: undefined, structure: undefined };
  }

  function broadcastOSCState(rustState: RustAudioEngineStateUpdate) {
    if (!oscManager) return;
    if (rustState.masterTempo && rustState.masterTempo !== lastOSCTempo) {
      oscManager.sendMasterTempo(rustState.masterTempo);
      lastOSCTempo = rustState.masterTempo;
    }
    const nextA = deckATrack?.id ?? null;
    if (nextA !== lastOSCDeckATrackId) {
      oscManager.sendCurrentTrack(deckATrack, 'A');
      lastOSCDeckATrackId = nextA;
    }
    const nextB = deckBTrack?.id ?? null;
    if (nextB !== lastOSCDeckBTrackId) {
      oscManager.sendCurrentTrack(deckBTrack, 'B');
      lastOSCDeckBTrackId = nextB;
    }
  }

  function convertRustState(rustState: RustAudioEngineStateUpdate): AudioEngineState {
    broadcastOSCState(rustState);
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
      deckALoop: rustState.deckALoop.enabled
        ? { ...rustState.deckALoop, beats: deckALoopBeats ?? 0 }
        : undefined,
      deckBLoop: rustState.deckBLoop.enabled
        ? { ...rustState.deckBLoop, beats: deckBLoopBeats ?? 0 }
        : undefined,
      isSeek: rustState.updateReason === 'seek',
      micAvailable: rustState.micAvailable,
      micEnabled: rustState.micEnabled,
      micWarning: null,
      talkoverActive: false,
      talkoverButtonPressed: false,
      micLevel: rustState.micPeak,
      sampleRate: rustState.sampleRate,
      deckATotalFrames: rustState.deckATotalFrames,
      deckBTotalFrames: rustState.deckBTotalFrames,
    };
  }

  function updateAudioCaches(state: AudioEngineState) {
    cachedAudioState = state;
    cachedLevelState = {
      deckAPeak: state.deckAPeak,
      deckBPeak: state.deckBPeak,
      deckAPeakHold: state.deckAPeakHold,
      deckBPeakHold: state.deckBPeakHold,
      micLevel: state.micLevel ?? 0,
      talkoverActive: state.talkoverActive ?? false,
      talkoverButtonPressed: state.talkoverButtonPressed ?? false,
    };
    if (state.masterTempo != null) cachedMasterTempo = state.masterTempo;
    if (state.deckAPosition != null) cachedDeckAPosition = state.deckAPosition;
    if (state.deckBPosition != null) cachedDeckBPosition = state.deckBPosition;
    if (state.sampleRate != null) cachedSampleRate = state.sampleRate;
    if (state.deckA?.id) deckATrackId = state.deckA.id;
    if (state.deckB?.id) deckBTrackId = state.deckB.id;
  }

  function handleWaveformChunk(trackId: string, chunkIndex: number, totalChunks: number, chunk: number[]) {
    const buffer = nativeWaveformBuffers.get(trackId) ?? {
      totalChunks,
      compactChunks: new Array<number[] | undefined>(totalChunks),
    };
    if (buffer.totalChunks !== totalChunks || buffer.compactChunks.length !== totalChunks) {
      buffer.totalChunks = totalChunks;
      buffer.compactChunks = new Array<number[] | undefined>(totalChunks);
    }
    if (chunkIndex >= 0 && chunkIndex < buffer.totalChunks) {
      buffer.compactChunks[chunkIndex] = chunk;
    }
    nativeWaveformBuffers.set(trackId, buffer);
    callbacks.onWaveformChunk({ trackId, chunkIndex, totalChunks, chunk });
  }

  function handleWaveformComplete(trackId: string, totalFrames: number) {
    const buffer = nativeWaveformBuffers.get(trackId);
    if (buffer) {
      let deck: 1 | 2 | null = null;
      if (deckATrackId === trackId) deck = 1;
      else if (deckBTrackId === trackId) deck = 2;

      if (deck !== null) {
        const flat: number[] = [];
        for (const chunk of buffer.compactChunks) {
          if (chunk) {
            for (let i = 0; i < chunk.length; i++) {
              flat.push(Math.abs(chunk[i] ?? 0));
            }
          }
        }
        if (flat.length > 0) {
          callbacks.onNativeWaveformReady(deck, flat);
        }
      }
      nativeWaveformBuffers.delete(trackId);
    }
    callbacks.onWaveformComplete({ trackId, totalFrames });
  }

  async function emitWaveform(trackId: string, waveformData: Float32Array | number[]) {
    const CHUNK_SIZE = 44100;
    const totalFrames = waveformData.length;
    const totalChunks = Math.ceil(totalFrames / CHUNK_SIZE);
    for (let i = 0; i < totalChunks; i += 1) {
      const start = i * CHUNK_SIZE;
      const end = Math.min(start + CHUNK_SIZE, totalFrames);
      const chunk = Array.from(waveformData.slice(start, end));
      handleWaveformChunk(trackId, i, totalChunks, chunk);
      await new Promise<void>((resolve) => setImmediate(resolve));
    }
    handleWaveformComplete(trackId, totalFrames);
  }

  async function ensureAudioModule(): Promise<RustAudioModule> {
    if (!audioModule) {
      audioModule = (await import('@sujay/audio')) as unknown as RustAudioModule;
      decodeAudio = audioModule.decodeAudio;
    }
    return audioModule;
  }

  function decodeTrack(track: Track): {
    pcmData: Float32Array;
    waveformData: Float32Array;
    bpm?: number;
    structure?: TrackStructure;
  } {
    if (!track.mp3Path) throw new Error('Track mp3Path missing');
    if (!decodeAudio) throw new Error('Decoder not initialized');
    const result = decodeAudio(track.mp3Path, AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
    const pcmData = new Float32Array(result.pcm.buffer, result.pcm.byteOffset, result.pcm.byteLength / 4);
    const waveformData = new Float32Array(result.mono.buffer, result.mono.byteOffset, result.mono.byteLength / 4);
    const structure = result.structure
      ? {
        bpm: result.structure.bpm,
        beats: result.structure.beats ?? [],
        intro: result.structure.intro,
        main: result.structure.main,
        outro: result.structure.outro,
        hotCues: result.structure.hotCues ?? [],
      }
      : undefined;
    return { pcmData, waveformData, bpm: result.bpm, structure };
  }

  async function loadTrackToDeck(track: Track, deck: 1 | 2) {
    if (!audioEngine) throw new Error('AudioEngine not initialized');
    let pcmData = track.pcmData;
    let bpm = track.bpm;
    let waveformData = track.waveformData;
    let structure = track.structure;

    if (!pcmData) {
      const decoded = decodeTrack(track);
      pcmData = decoded.pcmData;
      bpm = decoded.bpm;
      waveformData = decoded.waveformData;
      structure = decoded.structure;
    }
    if (!pcmData) throw new Error('PCM data is required');

    audioEngine.loadTrack(deck, pcmData, bpm, track.id);

    const trackWithData: Track = { ...track, pcmData, bpm, waveformData, structure };
    if (deck === 1) {
      deckATrack = trackWithData;
      deckATrackId = track.id;
    } else {
      deckBTrack = trackWithData;
      deckBTrackId = track.id;
    }

    if (waveformData) {
      await emitWaveform(track.id, waveformData);
    }
    if (structure) {
      trackStructureMap.set(track.id, structure);
      callbacks.onTrackStructure({ trackId: track.id, deck, structure });
    }
  }

  function internalSetBeatLoop(deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) {
    if (!audioEngine) throw new Error('AudioEngine not initialized');
    let startSeconds: number;
    let endSeconds: number;

    if (beatGrid && beatGrid.length > 0) {
      let startBeatIndex = 0;
      for (let i = 0; i < beatGrid.length; i += 1) {
        if (beatGrid[i] <= currentPosition) startBeatIndex = i;
        else break;
      }
      startSeconds = beatGrid[startBeatIndex];
      if (beats < 1) {
        const beatDuration = startBeatIndex + 1 < beatGrid.length
          ? beatGrid[startBeatIndex + 1] - beatGrid[startBeatIndex]
          : 60.0 / masterTempo;
        endSeconds = startSeconds + beatDuration * beats;
      } else {
        const endBeatIndex = startBeatIndex + beats;
        if (endBeatIndex < beatGrid.length) {
          endSeconds = beatGrid[endBeatIndex];
        } else {
          endSeconds = startSeconds + (60.0 / masterTempo) * beats;
        }
      }
    } else {
      const secondsPerBeat = 60.0 / masterTempo;
      const beatNumber = Math.floor(currentPosition / secondsPerBeat);
      startSeconds = beatNumber * secondsPerBeat;
      endSeconds = startSeconds + secondsPerBeat * beats;
    }

    audioEngine.setBeatLoop(deck, startSeconds, endSeconds);
    if (deck === 1) deckALoopBeats = beats;
    else deckBLoopBeats = beats;
  }

  function applyAudioConfigInternal(config: AudioConfig) {
    if (!audioEngine) throw new Error('AudioEngine not initialized');
    const mainChannels = config.mainChannels ?? [0, 1];
    const cueChannels = config.cueChannels ?? [null, null];
    audioEngine.configureDevice({
      deviceId: config.deviceId,
      mainChannels: mainChannels.map((c) => c ?? -1),
      cueChannels: cueChannels.map((c) => c ?? -1),
    });
  }

  function setRecordingStatusInternal(next: RecordingStatus) {
    recordingStatus = next;
    callbacks.onRecordingStatus(recordingStatus);
  }

  // --- Public interface ------------------------------------------------------

  return {
    async initialize() {
      if (audioEngine) return;
      const mod = await ensureAudioModule();
      audioEngine = new mod.AudioEngine(
        null,
        AUDIO_CHANNELS,
        AUDIO_SAMPLE_RATE,
        (rustState: RustAudioEngineStateUpdate) => {
          const state = convertRustState(rustState);
          updateAudioCaches(state);
          callbacks.onStateUpdate(state, cachedLevelState);
        },
      );
      applyAudioConfigInternal(settingsStore.getAudioConfig());
      const oscConfig = settingsStore.getOscConfig();
      if (!oscManager) {
        oscManager = new OSCManager(oscConfig);
      } else {
        oscManager.updateConfig(oscConfig);
      }
    },

    async loadTrack(track, deck) {
      await loadTrackToDeck(track, deck);
    },

    async play(track, crossfade, targetDeck) {
      if (!audioEngine) throw new Error('AudioEngine not initialized');
      const deck = targetDeck ?? (deckATrack ? 2 : 1);
      await loadTrackToDeck(track, deck);
      if (crossfade && (deckATrack || deckBTrack)) {
        audioEngine.startCrossfade(deck === 2 ? 1 : 0, 2);
      }
      audioEngine.play(deck);
    },

    stop(deck) { audioEngine?.stop(deck); },
    seek(deck, position) { audioEngine?.seek(deck, position); },
    setCrossfader(position) { audioEngine?.setCrossfaderPosition(position); },
    setMasterTempo(bpm) { audioEngine?.setMasterTempo(bpm); },
    setDeckCue(deck, enabled) { audioEngine?.setDeckCueEnabled(deck, enabled); },
    setEqCut(deck, band, enabled) { audioEngine?.setEqCut(deck, band, enabled); },
    setDeckGain(deck, gain) { audioEngine?.setDeckGain(deck, gain); },
    setBeatLoop(deck, beats, masterTempo, currentPosition, beatGrid) {
      internalSetBeatLoop(deck, beats, masterTempo, currentPosition, beatGrid);
    },
    clearLoop(deck) {
      audioEngine?.clearLoop(deck);
      if (deck === 1) deckALoopBeats = null;
      else deckBLoopBeats = null;
    },
    startDeck(deck) { audioEngine?.play(deck); },
    setMicEnabled(enabled) { audioEngine?.setMicEnabled(enabled); },

    async listDevices() {
      const mod = await ensureAudioModule();
      return mod.listAudioDevices().filter((d) => (d.maxOutputChannels ?? 0) > 0);
    },

    applyAudioConfig(config) { applyAudioConfigInternal(config); },

    updateOscConfig(config) {
      if (!oscManager) {
        oscManager = new OSCManager(config);
      } else {
        oscManager.updateConfig(config);
      }
    },

    getRecordingStatus() { return recordingStatus; },

    async startRecording(format) {
      if (recordingStatus.state === 'recording' || recordingStatus.state === 'preparing') {
        return recordingStatus;
      }
      const config = settingsStore.getRecordingConfig();
      try {
        await ensureRecordingDirectory(config);
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Failed to prepare recording directory';
        setRecordingStatusInternal({ state: 'error', lastError: message });
        throw error instanceof Error ? error : new Error(message);
      }
      const fileInfo = await prepareRecordingFile(config, format);
      setRecordingStatusInternal({ state: 'preparing', activeFile: fileInfo, lastError: undefined });
      try {
        if (!audioEngine) throw new Error('AudioEngine not initialized');
        audioEngine.startRecording(fileInfo.path, format);
        setRecordingStatusInternal({ state: 'recording', activeFile: fileInfo, lastError: undefined });
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Failed to start recording';
        setRecordingStatusInternal({ state: 'error', lastError: message });
        throw error instanceof Error ? error : new Error(message);
      }
      return recordingStatus;
    },

    async stopRecording() {
      if (
        recordingStatus.state !== 'recording' &&
        recordingStatus.state !== 'preparing' &&
        recordingStatus.state !== 'stopping'
      ) {
        return recordingStatus;
      }
      const activeFile = recordingStatus.activeFile;
      setRecordingStatusInternal({ state: 'stopping', activeFile, lastError: undefined });
      try {
        if (!audioEngine) throw new Error('AudioEngine not initialized');
        audioEngine.stopRecording();
        setRecordingStatusInternal({ state: 'idle', activeFile: undefined, lastError: undefined });
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Failed to stop recording';
        setRecordingStatusInternal({ state: 'error', activeFile, lastError: message });
        throw error instanceof Error ? error : new Error(message);
      }
      return recordingStatus;
    },

    // State accessors
    getCachedState() { return cachedAudioState; },
    getCachedLevelState() { return cachedLevelState; },
    getCachedSampleRate() { return cachedSampleRate; },
    getCachedMasterTempo() { return cachedMasterTempo; },
    getDeckAPosition() { return cachedDeckAPosition; },
    getDeckBPosition() { return cachedDeckBPosition; },
    getDeckATotalFrames() { return cachedAudioState.deckATotalFrames ?? 0; },
    getDeckBTotalFrames() { return cachedAudioState.deckBTotalFrames ?? 0; },
    getDeckATrack() { return deckATrack; },
    getDeckBTrack() { return deckBTrack; },
    getDeckALoopBeats() { return deckALoopBeats; },
    getDeckBLoopBeats() { return deckBLoopBeats; },
    getDeckATrackId() { return deckATrackId; },
    getDeckBTrackId() { return deckBTrackId; },
    getTrackStructure(trackId) { return trackStructureMap.get(trackId); },

    close() {
      try {
        if (
          recordingStatus.state === 'recording' ||
          recordingStatus.state === 'preparing' ||
          recordingStatus.state === 'stopping'
        ) {
          audioEngine?.stopRecording();
          setRecordingStatusInternal({ state: 'idle' });
        }
      } catch {
        // best-effort on close
      }
      try {
        audioEngine?.close();
      } catch (error) {
        console.error('[AudioRuntime] engine close failed:', error);
      }
      audioEngine = null;
      audioModule = null;
      decodeAudio = null;
    },
  };
}
