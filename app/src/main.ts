import { app, BrowserWindow, ipcMain, Menu } from 'electron';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { createRequire } from 'node:module';
import started from 'electron-squirrel-startup';
import { IPC_CHANNELS, IPC_EVENTS } from './main/ipc-contract';
import { createElectronIpcHost } from './main/host/electron-ipc-host';
import { createElectronShellHost } from './main/host/shell-host';
import { createAppSettingsStore } from './main/settings/app-settings-store';
import { registerRuntimeLifecycle } from './main/runtime/lifecycle-bootstrap';

import type {
  AudioEngineState,
  AudioLevelState,
  OSCConfig,
  AudioConfig,
  RecordingConfig,
  RecordingStatus,
  RecordingFileInfo,
  Track,
  TrackStructure,
} from './types';
import { OSCManager } from './workers/osc-manager';

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

declare const MAIN_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const MAIN_WINDOW_VITE_NAME: string;
declare const PREFERENCES_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const PREFERENCES_WINDOW_VITE_NAME: string;

// Handle creating/removing shortcuts on Windows when installing/uninstalling.
const shellHost = createElectronShellHost({ app, BrowserWindow, Menu });

if (started) {
  shellHost.quit();
}

const defaultRecordingDirectory = path.join(shellHost.getPath('music'), 'Sujay Recordings');
const defaultRecordingConfig: RecordingConfig = {
  directory: defaultRecordingDirectory,
  autoCreateDirectory: true,
  namingStrategy: 'timestamp',
  format: 'wav',
};
const settingsStore = createAppSettingsStore(defaultRecordingConfig);

let mainWindow: BrowserWindow | null = null;
let preferencesWindow: BrowserWindow | null = null;
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
const ipcHost = createElectronIpcHost({
  ipcMain,
  getMainWindow: () => mainWindow,
});
let recordingStatus: RecordingStatus = { state: 'idle' };
let deckATrackId: string | null = null;
let deckBTrackId: string | null = null;
let cachedMasterTempo = 130;
let cachedDeckAPosition = 0;
let cachedDeckBPosition = 0;
let cachedSampleRate = 44100;
const trackStructureMap = new Map<string, TrackStructure>();

type WaveformChunkBuffer = {
  totalChunks: number;
  compactChunks: Array<number[] | undefined>;
};

const nativeWaveformBuffers = new Map<string, WaveformChunkBuffer>();

type NativeUIFrame = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type NativeDeckConsoleState = {
  title: string;
  timeText: string;
  bpmText: string;
  bpm: number;
  playing: boolean;
  loopEnabled: boolean;
  loopBeats: number;
  loopStart: number;
  loopEnd: number;
  cueEnabled: boolean;
  eqLow: boolean;
  eqMid: boolean;
  eqHigh: boolean;
  gain: number;
  peak: number;
};

type NativeConsoleState = {
  deckA: NativeDeckConsoleState;
  deckB: NativeDeckConsoleState;
  masterTempo: number;
  crossfader: number;
};

type NativeUiAction = {
  action: string;
  deck: number;
  value: number;
  param: string;
};

type NativeUIModule = {
  attach(nativeHandle: Buffer, x: number, y: number, width: number, height: number): void;
  isGpuiEnabled?(): boolean;
  launchGpuiPreview?(): boolean;
  setFrame(x: number, y: number, width: number, height: number): void;
  setWaveform(deck: number, samples: number[]): void;
  setDeckProgress?(deck: number, positionFrames: number, totalFrames: number, audioSampleRate: number): void;
  setDeckMarkers?(deck: number, beats: number[], intro: number | null, outro: number | null): void;
  setConsoleState?(state: NativeConsoleState): void;
  setDeckArtwork?(deck: number, width: number, height: number, rgba: Buffer): void;
  clearDeckArtwork?(deck: number): void;
  pollActions?(): NativeUiAction[];
  detach(): void;
};

const nodeRequire = createRequire(__filename);
let nativeUI: NativeUIModule | null = null;
let gpuiPreviewBooted = false;
const TARGET_SAMPLE_RATE = 44100;
const TARGET_CHANNELS = 2;

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

const stripTrackData = (track: Track | null): Track | undefined => {
  if (!track) return undefined;
  return { ...track, pcmData: undefined, waveformData: undefined, structure: undefined };
};

const broadcastOSCState = (rustState: RustAudioEngineStateUpdate) => {
  if (!oscManager) return;

  if (rustState.masterTempo && rustState.masterTempo !== lastOSCTempo) {
    oscManager.sendMasterTempo(rustState.masterTempo);
    lastOSCTempo = rustState.masterTempo;
  }

  const nextDeckATrackId = deckATrack?.id ?? null;
  if (nextDeckATrackId !== lastOSCDeckATrackId) {
    oscManager.sendCurrentTrack(deckATrack, 'A');
    lastOSCDeckATrackId = nextDeckATrackId;
  }

  const nextDeckBTrackId = deckBTrack?.id ?? null;
  if (nextDeckBTrackId !== lastOSCDeckBTrackId) {
    oscManager.sendCurrentTrack(deckBTrack, 'B');
    lastOSCDeckBTrackId = nextDeckBTrackId;
  }
};

const convertRustState = (rustState: RustAudioEngineStateUpdate): AudioEngineState => {
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
    deckALoop: rustState.deckALoop.enabled ? { ...rustState.deckALoop, beats: deckALoopBeats ?? 0 } : undefined,
    deckBLoop: rustState.deckBLoop.enabled ? { ...rustState.deckBLoop, beats: deckBLoopBeats ?? 0 } : undefined,
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
};

const formatNativeDeckTime = (positionFrames: number, durationSeconds: number, sampleRate: number): string => {
  const positionSeconds = sampleRate > 0 ? positionFrames / sampleRate : 0;
  const safePosition = Math.max(0, Math.min(positionSeconds, durationSeconds || 0));
  const mins = Math.floor(safePosition / 60);
  const secs = Math.floor(safePosition % 60).toString().padStart(2, '0');
  const durationMins = Math.floor((durationSeconds || 0) / 60);
  const durationSecs = Math.floor((durationSeconds || 0) % 60).toString().padStart(2, '0');
  return `${mins}:${secs} / ${durationMins}:${durationSecs}`;
};

const buildNativeDeckConsoleState = (
  track: Track | null | undefined,
  positionFrames: number,
  playing: boolean,
  loopState: AudioEngineState['deckALoop'] | AudioEngineState['deckBLoop'],
  cueEnabled: boolean,
  eqCut: AudioEngineState['deckAEqCut'] | AudioEngineState['deckBEqCut'],
  gain: number,
  peak: number,
  sampleRate: number,
): NativeDeckConsoleState => ({
  title: track?.title ?? '',
  timeText: formatNativeDeckTime(positionFrames, track?.duration ?? 0, sampleRate),
  bpmText: track?.bpm ? `${Math.round(track.bpm)}` : '',
  bpm: track?.bpm ?? 0,
  playing,
  loopEnabled: Boolean(loopState?.enabled && typeof loopState?.beats === 'number'),
  loopBeats: typeof loopState?.beats === 'number' ? loopState.beats : 0,
  loopStart: loopState?.enabled ? (loopState.start ?? 0) : 0,
  loopEnd: loopState?.enabled ? (loopState.end ?? 0) : 0,
  cueEnabled,
  eqLow: eqCut?.low ?? false,
  eqMid: eqCut?.mid ?? false,
  eqHigh: eqCut?.high ?? false,
  gain,
  peak,
});

const pushNativeConsoleState = () => {
  const sampleRate = cachedAudioState.sampleRate ?? cachedSampleRate ?? 44100;
  const consoleState: NativeConsoleState = {
    deckA: buildNativeDeckConsoleState(
      cachedAudioState.deckA,
      cachedAudioState.deckAPosition ?? 0,
      cachedAudioState.deckAPlaying ?? false,
      cachedAudioState.deckALoop,
      cachedAudioState.deckACueEnabled ?? false,
      cachedAudioState.deckAEqCut,
      cachedAudioState.deckAGain ?? 1.0,
      cachedLevelState.deckAPeak,
      sampleRate,
    ),
    deckB: buildNativeDeckConsoleState(
      cachedAudioState.deckB,
      cachedAudioState.deckBPosition ?? 0,
      cachedAudioState.deckBPlaying ?? false,
      cachedAudioState.deckBLoop,
      cachedAudioState.deckBCueEnabled ?? false,
      cachedAudioState.deckBEqCut,
      cachedAudioState.deckBGain ?? 1.0,
      cachedLevelState.deckBPeak,
      sampleRate,
    ),
    masterTempo: cachedAudioState.masterTempo ?? cachedMasterTempo,
    crossfader: cachedAudioState.crossfaderPosition ?? 0.5,
  };

  withNativeUI((native) => {
    if (typeof native.setConsoleState === 'function') {
      native.setConsoleState(consoleState);
    }
    return true;
  }, false);
};

const pushNativeDeckProgress = () => {
  const sampleRate = cachedAudioState.sampleRate ?? cachedSampleRate ?? 44100;
  withNativeUI((native) => {
    if (typeof native.setDeckProgress === 'function') {
      native.setDeckProgress(1, cachedAudioState.deckAPosition ?? 0, cachedAudioState.deckATotalFrames ?? 0, sampleRate);
      native.setDeckProgress(2, cachedAudioState.deckBPosition ?? 0, cachedAudioState.deckBTotalFrames ?? 0, sampleRate);
    }
    return true;
  }, false);
};

const pushNativeDeckMarkers = () => {
  const sampleRate = cachedAudioState.sampleRate ?? cachedSampleRate ?? 44100;
  const deckStates = [
    { deck: 1 as const, trackId: cachedAudioState.deckA?.id ?? deckATrackId },
    { deck: 2 as const, trackId: cachedAudioState.deckB?.id ?? deckBTrackId },
  ];

  withNativeUI((native) => {
    if (typeof native.setDeckMarkers === 'function') {
      for (const { deck, trackId } of deckStates) {
        const structure = trackId ? trackStructureMap.get(trackId) : undefined;
        const beats = (structure?.beats ?? []).map((beat) => beat * sampleRate);
        const intro = structure?.intro?.end != null ? structure.intro.end * sampleRate : null;
        const outro = structure?.outro?.start != null ? structure.outro.start * sampleRate : null;
        native.setDeckMarkers(deck, beats, intro, outro);
      }
    }
    return true;
  }, false);
};

const pushNativeUiState = () => {
  pushNativeConsoleState();
  pushNativeDeckProgress();
  pushNativeDeckMarkers();
};

const getNativeUI = (): NativeUIModule | null => {
  if (process.platform !== 'darwin') {
    return null;
  }
  if (nativeUI) {
    return nativeUI;
  }
  try {
    nativeUI = nodeRequire('@sujay/ui') as NativeUIModule;
    if (!gpuiPreviewBooted && process.env.SUJAY_GPUI_PREVIEW === '1') {
      gpuiPreviewBooted = true;
      try {
        if (nativeUI.isGpuiEnabled?.()) {
          const launched = nativeUI.launchGpuiPreview?.() ?? false;
          console.log(`[native-ui] gpui preview launch ${launched ? 'started' : 'skipped'}`);
        } else {
          console.log('[native-ui] gpui preview requested but not enabled in native build');
        }
      } catch (previewError) {
        console.error('[native-ui] gpui preview launch failed:', previewError);
      }
    }
    return nativeUI;
  } catch (error) {
    console.error('[native-ui] failed to load module:', error);
    return null;
  }
};

const sanitizeNativeUIFrame = (frame: NativeUIFrame): NativeUIFrame => ({
  x: Math.max(0, Math.round(frame.x)),
  y: Math.max(0, Math.round(frame.y)),
  width: Math.max(0, Math.round(frame.width)),
  height: Math.max(0, Math.round(frame.height)),
});

const withNativeUI = <T>(fn: (native: NativeUIModule) => T, fallback: T): T => {
  const native = getNativeUI();
  if (!native) {
    return fallback;
  }
  try {
    return fn(native);
  } catch (error) {
    console.error('[native-ui] operation failed:', error);
    return fallback;
  }
};

const createWindow = () => { 
  // Create the browser window
  mainWindow = shellHost.createBrowserWindow({
    width: 1100,
    height: 540,
    minWidth: 980,
    minHeight: 500,
    titleBarStyle: 'hidden',
    trafficLightPosition: { x: 10, y: 10 },
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
    },
  });

  mainWindow.on('closed', () => {
    withNativeUI((native) => {
      stopNativeUIPolling();
      native.detach();
      return true;
    }, false);
    mainWindow = null;
    if (preferencesWindow && !preferencesWindow.isDestroyed()) {
      preferencesWindow.close();
    }
    preferencesWindow = null;
  });

  // Create application menu
  const isMac = shellHost.platform === 'darwin';
  const template: Electron.MenuItemConstructorOptions[] = [];
  const SEP: Electron.MenuItemConstructorOptions = { type: 'separator' };

  if (isMac) {
    template.push({
      label: shellHost.appName,
      submenu: [
        {
          label: 'Preferences...',
          accelerator: 'CmdOrCtrl+,',
          click: () => createPreferencesWindow(),
        },
        SEP,
        { role: 'hide' },
        { role: 'hideOthers' },
        { role: 'unhide' },
        SEP,
        { role: 'quit' },
      ],
    });
  }

  template.push({
    label: 'Edit',
    submenu: (() => {
      const editSub: Electron.MenuItemConstructorOptions[] = [
        { role: 'undo' },
        { role: 'redo' },
        SEP,
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
      ];
      if (isMac) {
        editSub.push({ role: 'pasteAndMatchStyle' }, { role: 'delete' }, { role: 'selectAll' });
      } else {
        editSub.push({ role: 'delete' }, SEP, { role: 'selectAll' });
      }
      return editSub;
    })(),
  });

  template.push({
    label: 'View',
    submenu: [
      { role: 'reload' },
      { role: 'forceReload' },
      { role: 'toggleDevTools' },
      SEP,
      { role: 'resetZoom' },
      { role: 'zoomIn' },
      { role: 'zoomOut' },
      SEP,
      { role: 'togglefullscreen' },
    ],
  });

  const menu = shellHost.buildMenuFromTemplate(template);
  shellHost.setApplicationMenu(menu);

  // Load the index.html of the app
  if (MAIN_WINDOW_VITE_DEV_SERVER_URL) {
    mainWindow.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL);
  } else {
    mainWindow.loadFile(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`));
  }

  // Open DevTools in development
  // if (process.env.NODE_ENV === 'development') {
  //   mainWindow.webContents.openDevTools();
  // }
};

const createPreferencesWindow = () => {
  if (!mainWindow) {
    return;
  }

  if (preferencesWindow && !preferencesWindow.isDestroyed()) {
    preferencesWindow.focus();
    return;
  }

  preferencesWindow = shellHost.createBrowserWindow({
    parent: mainWindow,
    modal: true,
    width: 520,
    height: 580,
    resizable: false,
    minimizable: false,
    maximizable: false,
    show: false,
    autoHideMenuBar: true,
    title: 'Preferences',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
    },
  });

  preferencesWindow.on('closed', () => {
    preferencesWindow = null;
  });

  if (PREFERENCES_WINDOW_VITE_DEV_SERVER_URL) {
    const devUrl = new URL('preferences.html', PREFERENCES_WINDOW_VITE_DEV_SERVER_URL);
    preferencesWindow.loadURL(devUrl.toString());
  } else {
    preferencesWindow.loadFile(path.join(
      __dirname,
      `../renderer/${PREFERENCES_WINDOW_VITE_NAME}/preferences.html`,
    ));
  }

  preferencesWindow.once('ready-to-show', () => {
    preferencesWindow?.show();
  });
};

async function ensureAudioModule(): Promise<RustAudioModule> {
  if (!audioModule) {
    audioModule = (await import('@sujay/audio')) as unknown as RustAudioModule;
    decodeAudio = audioModule.decodeAudio;
  }
  return audioModule;
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

  if (state.deckA?.id) {
    deckATrackId = state.deckA.id;
  }
  if (state.deckB?.id) {
    deckBTrackId = state.deckB.id;
  }
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
  sendToRenderer('waveform-chunk', { trackId, chunkIndex, totalChunks, chunk });
}

function handleWaveformComplete(trackId: string, totalFrames: number) {
  const buffer = nativeWaveformBuffers.get(trackId);
  if (buffer) {
    let deck: 1 | 2 | null = null;
    if (deckATrackId === trackId) {
      deck = 1;
    } else if (deckBTrackId === trackId) {
      deck = 2;
    }

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
        withNativeUI((native) => {
          native.setWaveform(deck as 1 | 2, flat);
          return true;
        }, false);
      }
    }

    nativeWaveformBuffers.delete(trackId);
  }
  sendToRenderer('waveform-complete', { trackId, totalFrames });
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

function decodeTrack(track: Track): { pcmData: Float32Array; waveformData: Float32Array; bpm?: number; structure?: TrackStructure } {
  if (!track.mp3Path) {
    throw new Error('Track mp3Path missing');
  }
  if (!decodeAudio) {
    throw new Error('Decoder not initialized');
  }

  const result = decodeAudio(track.mp3Path, TARGET_SAMPLE_RATE, TARGET_CHANNELS);
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

  return {
    pcmData,
    waveformData,
    bpm: result.bpm,
    structure,
  };
}

async function loadTrackToDeck(track: Track, deck: 1 | 2) {
  if (!audioEngine) {
    throw new Error('AudioEngine not initialized');
  }

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

  if (!pcmData) {
    throw new Error('PCM data is required');
  }

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
    pushNativeDeckMarkers();
    sendToRenderer('track-structure', { trackId: track.id, deck, structure });
  }
}

function setBeatLoop(deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) {
  if (!audioEngine) {
    throw new Error('AudioEngine not initialized');
  }

  let startSeconds: number;
  let endSeconds: number;

  if (beatGrid && beatGrid.length > 0) {
    let startBeatIndex = 0;
    for (let i = 0; i < beatGrid.length; i += 1) {
      if (beatGrid[i] <= currentPosition) {
        startBeatIndex = i;
      } else {
        break;
      }
    }

    startSeconds = beatGrid[startBeatIndex];

    if (beats < 1) {
      let beatDuration: number;
      if (startBeatIndex + 1 < beatGrid.length) {
        beatDuration = beatGrid[startBeatIndex + 1] - beatGrid[startBeatIndex];
      } else {
        beatDuration = 60.0 / masterTempo;
      }
      endSeconds = startSeconds + (beatDuration * beats);
    } else {
      const endBeatIndex = startBeatIndex + beats;
      if (endBeatIndex < beatGrid.length) {
        endSeconds = beatGrid[endBeatIndex];
      } else {
        const secondsPerBeat = 60.0 / masterTempo;
        endSeconds = startSeconds + (secondsPerBeat * beats);
      }
    }
  } else {
    const secondsPerBeat = 60.0 / masterTempo;
    const beatNumber = Math.floor(currentPosition / secondsPerBeat);
    startSeconds = beatNumber * secondsPerBeat;
    endSeconds = startSeconds + (secondsPerBeat * beats);
  }

  audioEngine.setBeatLoop(deck, startSeconds, endSeconds);
  if (deck === 1) {
    deckALoopBeats = beats;
  } else {
    deckBLoopBeats = beats;
  }
}

function applyAudioConfig(config: AudioConfig) {
  if (!audioEngine) {
    throw new Error('AudioEngine not initialized');
  }
  const mainChannels = config.mainChannels ?? [0, 1];
  const cueChannels = config.cueChannels ?? [null, null];
  audioEngine.configureDevice({
    deviceId: config.deviceId,
    mainChannels: mainChannels.map((c) => c ?? -1),
    cueChannels: cueChannels.map((c) => c ?? -1),
  });
}

function updateOSCConfig(config: OSCConfig) {
  if (!oscManager) {
    oscManager = new OSCManager(config);
  } else {
    oscManager.updateConfig(config);
  }
}

async function initializeCore() {
  if (audioEngine) {
    return;
  }

  const mod = await ensureAudioModule();

  audioEngine = new mod.AudioEngine(
    null,
    2,
    TARGET_SAMPLE_RATE,
    (rustState: RustAudioEngineStateUpdate) => {
      const state = convertRustState(rustState);
      updateAudioCaches(state);
      pushNativeUiState();
    },
  );

  applyAudioConfig(settingsStore.getAudioConfig());
  updateOSCConfig(settingsStore.getOscConfig());
}

function setRecordingStatus(next: RecordingStatus) {
  recordingStatus = next;
  ipcHost.send(IPC_EVENTS.recordingStatus, recordingStatus);
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

async function pathExists(filePath: string) {
  try {
    await fs.access(filePath);
    return true;
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === 'ENOENT') {
      return false;
    }
    throw err;
  }
}

function recordingExtensionForFormat(format: 'wav' | 'ogg') {
  switch (format) {
    case 'ogg':
      return '.ogg';
    case 'wav':
    default:
      return '.wav';
  }
}
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

async function generateTimestampFilePath(directory: string, date: Date, extension: string) {
  const base = buildTimestampLabel(date);
  for (let suffix = 0; suffix < MAX_TIMESTAMP_SUFFIX; suffix += 1) {
    const suffixPart = suffix === 0 ? '' : `-${suffix}`;
    const candidate = path.join(directory, `${base}${suffixPart}${extension}`);
    if (!(await pathExists(candidate))) {
      return candidate;
    }
  }
  throw new Error('Unable to allocate timestamp-based recording filename (too many collisions)');
}

async function generateSequentialFilePath(directory: string, extension: string) {
  for (let index = 1; index < 10000; index += 1) {
    const candidate = path.join(directory, `${padNumber(index, 4)}${extension}`);
    if (!(await pathExists(candidate))) {
      return candidate;
    }
  }
  throw new Error('Unable to allocate recording filename (too many existing recordings)');
}

async function prepareRecordingFile(config: RecordingConfig, format: 'wav' | 'ogg'): Promise<RecordingFileInfo> {
  const createdAt = Date.now();
  const directory = config.directory;
  const ext = recordingExtensionForFormat(format);
  const filePath = config.namingStrategy === 'timestamp'
    ? await generateTimestampFilePath(directory, new Date(createdAt), ext)
    : await generateSequentialFilePath(directory, ext);

  return {
    path: filePath,
    createdAt,
    bytesWritten: 0,
  };
}

// IPC Handlers
ipcHost.handle(IPC_CHANNELS.audio.loadTrack, async (track, deck) => {
  await initializeCore();
  await loadTrackToDeck(track, deck);
});

ipcHost.handle(IPC_CHANNELS.audio.play, async (track, crossfade, targetDeck) => {
  await initializeCore();
  if (!audioEngine) {
    throw new Error('AudioEngine not initialized');
  }

  const deck = targetDeck ?? (deckATrack ? 2 : 1);
  await loadTrackToDeck(track, deck);

  if (crossfade && (deckATrack || deckBTrack)) {
    const targetPosition = deck === 2 ? 1 : 0;
    audioEngine.startCrossfade(targetPosition, 2);
  }

  audioEngine.play(deck);
});

ipcHost.handle(IPC_CHANNELS.audio.stop, async (deck) => {
  await initializeCore();
  audioEngine?.stop(deck);
});

ipcHost.handle(IPC_CHANNELS.audio.getState, async () => {
  await initializeCore();
  if (!audioEngine) {
    return cachedAudioState;
  }
  const state = convertRustState(audioEngine.getState());
  updateAudioCaches(state);
  return state;
});

ipcHost.handle(IPC_CHANNELS.audio.seek, async (deck, position) => {
  await initializeCore();
  audioEngine?.seek(deck, position);
});

ipcHost.handle(IPC_CHANNELS.audio.setCrossfader, async (position) => {
  await initializeCore();
  audioEngine?.setCrossfaderPosition(position);
});

ipcHost.handle(IPC_CHANNELS.audio.setMasterTempo, async (bpm) => {
  await initializeCore();
  audioEngine?.setMasterTempo(bpm);
});

ipcHost.handle(IPC_CHANNELS.audio.setDeckCue, async (deck, enabled) => {
  await initializeCore();
  audioEngine?.setDeckCueEnabled(deck, enabled);
});

ipcHost.handle(IPC_CHANNELS.audio.setEqCut, async (deck, band, enabled) => {
  await initializeCore();
  audioEngine?.setEqCut(deck, band, enabled);
});

ipcHost.handle(IPC_CHANNELS.audio.setDeckGain, async (deck, gain) => {
  await initializeCore();
  audioEngine?.setDeckGain(deck, gain);
});

ipcHost.handle(IPC_CHANNELS.audio.setBeatLoop, async (deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) => {
  await initializeCore();
  setBeatLoop(deck, beats, masterTempo, currentPosition, beatGrid);
});

ipcHost.handle(IPC_CHANNELS.audio.clearLoop, async (deck: 1 | 2) => {
  await initializeCore();
  audioEngine?.clearLoop(deck);
  if (deck === 1) {
    deckALoopBeats = null;
  } else {
    deckBLoopBeats = null;
  }
});

ipcHost.handle(IPC_CHANNELS.audio.startDeck, async (deck) => {
  await initializeCore();
  audioEngine?.play(deck);
});

ipcHost.handle(IPC_CHANNELS.audio.setMicEnabled, async (enabled) => {
  await initializeCore();
  audioEngine?.setMicEnabled(enabled);
});

// Audio device/config handlers
ipcHost.handle(IPC_CHANNELS.audio.getDevices, async () => {
  const mod = await ensureAudioModule();
  return mod.listAudioDevices().filter((d) => (d.maxOutputChannels ?? 0) > 0);
});

ipcHost.handle(IPC_CHANNELS.audio.getConfig, () => {
  return settingsStore.getAudioConfig();
});

ipcHost.handle(IPC_CHANNELS.audio.updateConfig, async (config: AudioConfig) => {
  settingsStore.setAudioConfig(config);
  await initializeCore();
  try {
    applyAudioConfig(config);
  } catch (error) {
    console.error('Failed to apply audio config in audio runtime:', error);
  }
});

// Recording config/state handlers
ipcHost.handle(IPC_CHANNELS.recording.getConfig, () => {
  return settingsStore.getRecordingConfig();
});

ipcHost.handle(IPC_CHANNELS.recording.updateConfig, (config: RecordingConfig) => {
  settingsStore.setRecordingConfig(config);
  return settingsStore.getRecordingConfig();
});

ipcHost.handle(IPC_CHANNELS.recording.getStatus, () => {
  return recordingStatus;
});

ipcHost.handle(IPC_CHANNELS.recording.start, async (format: 'wav' | 'ogg') => {
  if (recordingStatus.state === 'recording' || recordingStatus.state === 'preparing') {
    return recordingStatus;
  }

  const config = settingsStore.getRecordingConfig();
  try {
    await ensureRecordingDirectory(config);
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Failed to prepare recording directory';
    setRecordingStatus({ state: 'error', lastError: message });
    throw error instanceof Error ? error : new Error(message);
  }

  const fileInfo = await prepareRecordingFile(config, format);
  setRecordingStatus({ state: 'preparing', activeFile: fileInfo, lastError: undefined });

  try {
    await initializeCore();
    if (!audioEngine) {
      throw new Error('AudioEngine not initialized');
    }
    audioEngine.startRecording(fileInfo.path, format);
    setRecordingStatus({ state: 'recording', activeFile: fileInfo, lastError: undefined });
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Failed to start recording';
    setRecordingStatus({ state: 'error', lastError: message });
    throw error instanceof Error ? error : new Error(message);
  }
  return recordingStatus;
});

ipcHost.handle(IPC_CHANNELS.recording.stop, async () => {
  if (recordingStatus.state !== 'recording' && recordingStatus.state !== 'preparing' && recordingStatus.state !== 'stopping') {
    return recordingStatus;
  }

  const activeFile = recordingStatus.activeFile;
  setRecordingStatus({ state: 'stopping', activeFile, lastError: undefined });

  try {
    await initializeCore();
    if (!audioEngine) {
      throw new Error('AudioEngine not initialized');
    }
    audioEngine.stopRecording();
    setRecordingStatus({ state: 'idle', activeFile: undefined, lastError: undefined });
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Failed to stop recording';
    setRecordingStatus({ state: 'error', activeFile, lastError: message });
    throw error instanceof Error ? error : new Error(message);
  }
  return recordingStatus;
});

let nativeUIPollingTimer: ReturnType<typeof setInterval> | null = null;

const startNativeUIPolling = () => {
  if (nativeUIPollingTimer) return;
  nativeUIPollingTimer = setInterval(() => {
    const native = getNativeUI();
    if (!native?.pollActions || !audioEngine) return;
    const actions = native.pollActions();
    for (const a of actions) {
      const deck = a.deck;
      switch (a.action) {
        case 'play':
          audioEngine.play(deck);
          break;
        case 'stop':
          audioEngine.stop(deck);
          break;
        case 'crossfader':
          audioEngine.setCrossfaderPosition(a.value);
          break;
        case 'master_tempo':
          audioEngine.setMasterTempo(a.value);
          break;
        case 'cue': {
          audioEngine.setDeckCueEnabled(deck, a.value > 0.5);
          break;
        }
        case 'eq': {
          audioEngine.setEqCut(deck, a.param, a.value > 0.5);
          break;
        }
        case 'loop': {
          if (a.value <= 0) {
            audioEngine.clearLoop(deck);
            if (deck === 1) {
              deckALoopBeats = null;
            } else {
              deckBLoopBeats = null;
            }
          } else {
            const posFrames = deck === 1 ? cachedDeckAPosition : cachedDeckBPosition;
            const currentPosition = posFrames / cachedSampleRate;
            const trackId = deck === 1 ? deckATrackId : deckBTrackId;
            const beatGrid = trackId ? trackStructureMap.get(trackId)?.beats : undefined;
            setBeatLoop(deck, a.value, cachedMasterTempo, currentPosition, beatGrid);
          }
          break;
        }
        case 'seek':
          audioEngine.seek(deck, a.value);
          break;
        case 'deck_gain':
          audioEngine.setDeckGain(deck, a.value);
          break;
        case 'load_file': {
          const filePath = a.param?.trim();
          if (!filePath) {
            break;
          }
          const base = path.basename(filePath);
          const title = base.replace(/\.[^.]+$/, '') || base;
          const track: Track = {
            id: `local:${filePath}:${Date.now()}`,
            title,
            mp3Path: filePath,
            duration: 0,
          };
          void loadTrackToDeck(track, deck).catch((error) => {
            const message = error instanceof Error ? error.message : String(error);
            sendToRenderer('notification', `Audio Error: ${message}`);
          });
          break;
        }
      }
    }
  }, 50);
};

const stopNativeUIPolling = () => {
  if (nativeUIPollingTimer) {
    clearInterval(nativeUIPollingTimer);
    nativeUIPollingTimer = null;
  }
};

ipcHost.handle(IPC_CHANNELS.nativeUi.attach, async (frame: NativeUIFrame) => {
  const win = mainWindow;
  if (!win || win.isDestroyed()) {
    return false;
  }
  const next = sanitizeNativeUIFrame(frame);
  if (next.width <= 0 || next.height <= 0) {
    return false;
  }
  return withNativeUI((native) => {
    const handle = win.getNativeWindowHandle();
    native.attach(handle, next.x, next.y, next.width, next.height);
    startNativeUIPolling();
    // Ensure native console paints immediately after attach, even before next worker tick.
    pushNativeUiState();
    return true;
  }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.setFrame, async (frame: NativeUIFrame) => {
  const next = sanitizeNativeUIFrame(frame);
  if (next.width <= 0 || next.height <= 0) {
    return false;
  }
  return withNativeUI((native) => {
    native.setFrame(next.x, next.y, next.width, next.height);
    return true;
  }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.setWaveform, async (deck: 1 | 2, samples: number[]) => {
  return withNativeUI((native) => {
    native.setWaveform(deck, samples);
    return true;
  }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.detach, async () => {
  return withNativeUI((native) => {
    stopNativeUIPolling();
    native.detach();
    return true;
  }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.setArtwork, async (deck: 1 | 2, width: number, height: number, rgba: Buffer) => {
  return withNativeUI((native) => {
    if (typeof native.setDeckArtwork === 'function') {
      native.setDeckArtwork(deck, width, height, rgba);
    }
    return true;
  }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.clearArtwork, async (deck: 1 | 2) => {
  return withNativeUI((native) => {
    if (typeof native.clearDeckArtwork === 'function') {
      native.clearDeckArtwork(deck);
    }
    return true;
  }, false);
});

// System info
ipcHost.handle(IPC_CHANNELS.system.getInfo, () => {
  const metrics = shellHost.getAppMetrics();
  
  // Sum CPU usage across all processes
  const totalCpuPercent = metrics.reduce((sum, metric) => {
    return sum + metric.cpu.percentCPUUsage;
  }, 0);
  
  const time = new Date().toLocaleTimeString('en-US', { 
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  });
  
  return { time, cpuUsage: Math.round(totalCpuPercent * 10) / 10, memoryUsage: Math.round(process.memoryUsage().rss / 1024 / 1024) };
});

// OSC config handlers
ipcHost.handle(IPC_CHANNELS.osc.getConfig, () => {
  return settingsStore.getOscConfig();
});

ipcHost.handle(IPC_CHANNELS.osc.updateConfig, async (config: OSCConfig) => {
  settingsStore.setOscConfig(config);
  updateOSCConfig(config);
});


// App lifecycle
registerRuntimeLifecycle(shellHost, {
  onReady: async () => {
  await initializeCore();

  createWindow();
  },

  onWindowAllClosed: async () => {
  if (audioEngine) {
    try {
      audioEngine.close();
    } catch (error) {
      console.error('[AudioEngine] cleanup failed:', error);
    }
    audioEngine = null;
  }

  if (shellHost.platform !== 'darwin') {
    shellHost.quit();
  }
  },

  onActivate: () => {
    if (shellHost.shouldOpenMainWindow()) {
      createWindow();
    }
  },

// Ensure recording is finalized on explicit app quit (e.g., Cmd+Q on macOS)
  onBeforeQuit: async () => {
    try {
      if (recordingStatus.state === 'recording' || recordingStatus.state === 'preparing' || recordingStatus.state === 'stopping') {
        audioEngine?.stopRecording();
        setRecordingStatus({ state: 'idle', activeFile: undefined, lastError: undefined });
      }
    } catch (err) {
      console.error('[Recording] Failed to stop during before-quit:', err);
    }
  },
});

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and import them here.
