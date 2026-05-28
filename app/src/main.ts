import { app, BrowserWindow, ipcMain, Menu } from 'electron';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { createRequire } from 'node:module';
import { Worker as NodeWorker } from 'node:worker_threads';
import started from 'electron-squirrel-startup';
import Store from 'electron-store';
// Local minimal typed interface to avoid (any) casts while TS 4.5 cannot see inherited methods.
interface AppStoreSchema { osc: OSCConfig; audio: AudioConfig; recording: RecordingConfig; suno: SunoConfig }
interface AppStore {
  get(key: 'osc'): OSCConfig;
  get(key: 'audio'): AudioConfig;
  get(key: 'recording'): RecordingConfig;
  get(key: 'suno'): SunoConfig;
  set(key: 'osc', value: OSCConfig): void;
  set(key: 'audio', value: AudioConfig): void;
  set(key: 'recording', value: RecordingConfig): void;
  set(key: 'suno', value: SunoConfig): void;
}
type AppPathKey = Parameters<typeof app.getPath>[0];

import { LibraryManager } from './core/library-manager';
import { MCPController } from './core/controllers/mcp-controller';
import { startMcpServer, stopMcpServer } from './main/mcp-server';
import type {
  AudioEngineState,
  AudioLevelState,
  LibraryState,
  OSCConfig,
  AudioConfig,
  RecordingConfig,
  RecordingStatus,
  RecordingFileInfo,
  SunoConfig,
  Track,
  TrackStructure,
} from './types';
import type { WorkerInMsg, WorkerOutMsg } from './workers/audio-worker-types';

declare const MAIN_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const MAIN_WINDOW_VITE_NAME: string;
declare const PREFERENCES_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const PREFERENCES_WINDOW_VITE_NAME: string;

// Handle creating/removing shortcuts on Windows when installing/uninstalling.
if (started) {
  app.quit();
}

const sunoCacheDir = path.join(app.getPath('cache' as AppPathKey), app.getName(), 'Suno');
const defaultRecordingDirectory = path.join(app.getPath('music' as AppPathKey), 'Sujay Recordings');
const defaultRecordingConfig: RecordingConfig = {
  directory: defaultRecordingDirectory,
  autoCreateDirectory: true,
  namingStrategy: 'timestamp',
  format: 'wav',
};

// Initialize electron-store with schema
const storeRaw = new Store<AppStoreSchema>({
  defaults: {
    osc: {
      enabled: false,
      host: '127.0.0.1',
      port: 9000,
    },
    audio: {
      mainChannels: [0, 1],
      cueChannels: [null, null],
    },
    recording: defaultRecordingConfig,
    suno: {
      cookie: '',
    },
  },
  schema: {
    osc: {
      type: 'object',
      properties: {
        enabled: { type: 'boolean' },
        host: { type: 'string' },
        port: { type: 'number', minimum: 1, maximum: 65535 },
      },
      required: ['enabled', 'host', 'port'],
    },
    audio: {
      type: 'object',
      properties: {
        deviceId: { type: ['string', 'null'] },
        mainChannels: { type: 'array', items: { type: ['number', 'null'] }, minItems: 2, maxItems: 2 },
        cueChannels: { type: 'array', items: { type: ['number', 'null'] }, minItems: 2, maxItems: 2 },
      },
      required: ['mainChannels', 'cueChannels'],
    },
    recording: {
      type: 'object',
      properties: {
        directory: { type: 'string' },
        autoCreateDirectory: { type: 'boolean' },
        namingStrategy: { type: 'string', enum: ['timestamp', 'sequential'] },
        format: { type: 'string', enum: ['wav', 'ogg'] },
      },
      required: ['directory', 'autoCreateDirectory', 'namingStrategy', 'format'],
    },
    suno: {
      type: 'object',
      properties: {
        cookie: { type: 'string' },
      },
      required: ['cookie'],
    },
  },
  // Migration: ensure recording.format exists (default to 'wav') for pre-OGG configs
  migrations: {
    '>=0.0.0': (store) => {
      try {
        const rec = store.get('recording') as Partial<RecordingConfig> | undefined;
        if (!rec || typeof rec !== 'object') {
          store.set('recording', defaultRecordingConfig);
        } else if (rec.format !== 'wav' && rec.format !== 'ogg') {
          // Add default format while preserving other fields
          store.set('recording', { ...defaultRecordingConfig, ...rec, format: 'wav' });
        }
      } catch {
        // If store is unreadable, reset to defaults
        store.set('recording', defaultRecordingConfig);
      }
    },
  },
});
const store: AppStore = storeRaw as unknown as AppStore;

// Core modules
let libraryManager: LibraryManager | null = null;
let mcpController: MCPController | null = null;
let mainWindow: BrowserWindow | null = null;
let preferencesWindow: BrowserWindow | null = null;
let audioWorker: NodeWorker | null = null;
let recordingStatus: RecordingStatus = { state: 'idle' };
let deckAPlaying = false;
let deckBPlaying = false;
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

const createEmptyLibraryState = (): LibraryState => ({
  tracks: [],
  workspaces: [],
  selectedWorkspace: null,
  likedFilter: false,
  syncing: false,
});

let libraryStateCache: LibraryState = createEmptyLibraryState();
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

const sendToRenderer = (channel: string, ...args: unknown[]) => {
  if (!mainWindow || mainWindow.isDestroyed()) {
    return;
  }
  const webContents = mainWindow.webContents;
  if (!webContents || webContents.isDestroyed()) {
    return;
  }
  try {
    if (webContents.getURL()) {
      webContents.send(channel, ...args);
    }
  } catch (error) {
    if (error instanceof Error && error.message.includes('Render frame was disposed')) {
      return;
    }
    console.error(`Error sending to renderer (${channel}):`, error);
  }
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
  mainWindow = new BrowserWindow({
    width: 1200,
    height: 800,
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
  const isMac = process.platform === 'darwin';
  const template: Electron.MenuItemConstructorOptions[] = [];
  const SEP: Electron.MenuItemConstructorOptions = { type: 'separator' };

  if (isMac) {
    template.push({
      label: app.name,
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

  const menu = Menu.buildFromTemplate(template);
  Menu.setApplicationMenu(menu);

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

  preferencesWindow = new BrowserWindow({
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

const getSunoCookieFromStore = () => (store.get('suno')?.cookie ?? '').trim();

const attachLibraryManagerEvents = (manager: LibraryManager) => {
  manager.on('state-changed', (state: LibraryState) => {
    libraryStateCache = state;
    sendToRenderer('library-state-changed', state);
    sendToRenderer('download-progress-changed', manager.getDownloadProgress());
  });

  manager.on('sync-started', (data: { workspaceId: string | null }) => {
    sendToRenderer('library-sync-started', data);
  });

  manager.on('sync-progress', (data: { current: number; total: number }) => {
    sendToRenderer('library-sync-progress', data);
  });

  manager.on('sync-completed', (data: { workspaceId: string | null }) => {
    sendToRenderer('library-sync-completed', data);
  });

  manager.on('sync-failed', (data: { error: string }) => {
    sendToRenderer('library-sync-failed', data);
  });

  manager.on('download-started', (data: { id: string }) => {
    sendToRenderer('download-started', data);
  });

  manager.on('download-progress', (data: { id: string; percent: number }) => {
    sendToRenderer('download-progress', data);
  });

  manager.on('download-completed', (data: { id: string }) => {
    sendToRenderer('download-completed', data);
  });

  manager.on('download-failed', (data: { id: string; error: string }) => {
    sendToRenderer('download-failed', data);
  });

  manager.on('error', (error: Error) => {
    sendToRenderer('notification', `Library Error: ${error.message}`);
  });
};

async function configureLibraryManager(cookie: string): Promise<void> {
  if (libraryManager) {
    libraryManager.removeAllListeners();
    libraryManager = null;
  }

  const manager = new LibraryManager(sunoCacheDir, cookie);
  attachLibraryManagerEvents(manager);

  // Initialize always succeeds (falls back to cache-only mode on error)
  await manager.initialize();

  libraryManager = manager;
  libraryStateCache = manager.getState();
  sendToRenderer('library-state-changed', libraryStateCache);
  sendToRenderer('download-progress-changed', manager.getDownloadProgress());
}

const requireLibraryManager = (): LibraryManager => {
  if (!libraryManager) {
    throw new Error('Suno cookie is not configured. Update it from Preferences.');
  }
  return libraryManager;
};

// Helper: send message to worker and wait for response
function sendWorkerMessage<T extends WorkerOutMsg>(msg: WorkerInMsg, timeout = 5000): Promise<T> {
  return new Promise((resolve, reject) => {
    if (!audioWorker) {
      return reject(new Error('Audio worker not available'));
    }
    const id = msg.id || Date.now();
    const msgWithId = { ...msg, id };
    const handler = (outMsg: WorkerOutMsg) => {
      if ('id' in outMsg && outMsg.id === id) {
        audioWorker?.off('message', handler);
        resolve(outMsg as T);
      }
    };
    audioWorker.on('message', handler);
    try {
      audioWorker.postMessage(msgWithId);
    } catch (e) {
      audioWorker.off('message', handler);
      reject(e);
    }
    setTimeout(() => {
      audioWorker?.off('message', handler);
      reject(new Error('Worker message timeout'));
    }, timeout);
  });
}

function setRecordingStatus(next: RecordingStatus) {
  recordingStatus = next;
  sendToRenderer('recording-status', recordingStatus);
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

// Initialize core modules
async function initializeCore() {
  if (audioWorker) {
    // Initialize via worker
    const audioConfig = store.get('audio');
    const oscConfig = store.get('osc');
    const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'init', audioConfig, oscConfig });
    if (res.type === 'initResult' && !res.ok) {
      throw new Error(`Worker init failed: ${res.error}`);
    }
  }

  const cookie = getSunoCookieFromStore();
  try {
    await configureLibraryManager(cookie);
  } catch (error) {
    console.error('Failed to initialize Suno library:', error);
  }

  // Initialize MCP controller if library manager is available
  if (libraryManager) {
    mcpController = new MCPController(libraryManager, sendWorkerMessage);
  }
}

// IPC Handlers
ipcMain.handle('audio:load-track', async (_event, track, deck) => {
  if (track?.id) {
    if (deck === 1) {
      deckATrackId = track.id;
    } else {
      deckBTrackId = track.id;
    }
  }
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'loadTrack', track, deck });
  if (res.type === 'loadTrackResult' && !res.ok) {
    throw new Error(res.error || 'Load track failed');
  }
});

ipcMain.handle('audio:play', async (_event, track, crossfade, targetDeck) => {
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'play', track, crossfade, targetDeck });
  if (res.type === 'playResult' && !res.ok) {
    throw new Error(res.error || 'Play failed');
  }
});

ipcMain.handle('audio:stop', async (_event, deck) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'stop', deck });
});

ipcMain.handle('audio:get-state', async () => {
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'getState' });
  return res.type === 'stateResult' ? res.state : {};
});

ipcMain.handle('audio:seek', async (_event, deck, position) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'seek', deck, position });
});

ipcMain.handle('audio:set-crossfader', async (_event, position) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'setCrossfader', position });
});

ipcMain.handle('audio:set-master-tempo', async (_event, bpm) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'setMasterTempo', bpm });
});

ipcMain.handle('audio:set-deck-cue', async (_event, deck, enabled) => {
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'setDeckCue', deck, enabled });
  if (res.type === 'setDeckCueResult' && !res.ok) {
    throw new Error(res.error || 'Failed to update deck cue state');
  }
});

ipcMain.handle('audio:set-eq-cut', async (_event, deck, band, enabled) => {
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'setEqCut', deck, band, enabled });
  if (res.type === 'setEqCutResult' && !res.ok) {
    throw new Error(res.error || 'Failed to update EQ cut state');
  }
});

ipcMain.handle('audio:set-deck-gain', async (_event, deck, gain) => {
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'setDeckGain', deck, gain });
  if (res.type === 'setDeckGainResult' && !res.ok) {
    throw new Error(res.error || 'Failed to update deck gain');
  }
});

ipcMain.handle('audio:set-beat-loop', async (_event, deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) => {
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'setBeatLoop', deck, beats, masterTempo, currentPosition, beatGrid });
  if (res.type === 'setBeatLoopResult' && !res.ok) {
    throw new Error(res.error || 'Failed to set beat loop');
  }
});

ipcMain.handle('audio:clear-loop', async (_event, deck: 1 | 2) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'clearLoop', deck });
});

ipcMain.handle('audio:start-deck', async (_event, deck) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'startDeck', deck });
});

ipcMain.handle('audio:set-mic-enabled', async (_event, enabled) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'setMicEnabled', enabled });
});

// Audio device/config handlers
ipcMain.handle('audio:get-devices', () => {
  return new Promise((resolve, reject) => {
    const id = Date.now();
    const handler = (msg: WorkerOutMsg) => {
      if (msg && msg.type === 'devices' && msg.id === id) {
        audioWorker?.off('message', handler);
        resolve(msg.devices);
      }
    };
    if (!audioWorker) {
      reject(new Error('Audio worker not initialized'));
      return;
    }
    audioWorker.on('message', handler);
    try {
      audioWorker.postMessage({ type: 'getDevices', id });
    } catch (e) {
      audioWorker.off('message', handler);
      reject(e);
    }
    setTimeout(() => {
      audioWorker?.off('message', handler);
      reject(new Error('audio worker getDevices timeout'));
    }, 3000);
  });
});

ipcMain.handle('audio:get-config', () => {
  return store.get('audio');
});

ipcMain.handle('audio:update-config', async (_event, config: AudioConfig) => {
  store.set('audio', config);
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'applyAudioConfig', config });
  if (res.type === 'applyAudioConfigResult' && !res.ok) {
    console.error('Failed to apply audio config in worker:', res.error);
  }
});

// Recording config/state handlers
ipcMain.handle('recording:get-config', () => {
  return store.get('recording');
});

ipcMain.handle('recording:update-config', (_event, config: RecordingConfig) => {
  store.set('recording', config);
  return store.get('recording');
});

ipcMain.handle('recording:get-status', () => {
  return recordingStatus;
});

ipcMain.handle('recording:start', async (_event, format: 'wav' | 'ogg') => {
  if (recordingStatus.state === 'recording' || recordingStatus.state === 'preparing') {
    return recordingStatus;
  }

  const config = store.get('recording');
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
    const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'startRecording', path: fileInfo.path, format });
    if (res.type === 'startRecordingResult' && res.ok) {
      setRecordingStatus({ state: 'recording', activeFile: fileInfo, lastError: undefined });
    } else {
      const message = res.type === 'startRecordingResult' ? (res.error || 'Failed to start recording') : 'Unexpected worker response for recording start';
      throw new Error(message);
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Failed to start recording';
    setRecordingStatus({ state: 'error', lastError: message });
    throw error instanceof Error ? error : new Error(message);
  }
  return recordingStatus;
});

ipcMain.handle('recording:stop', async () => {
  if (recordingStatus.state !== 'recording' && recordingStatus.state !== 'preparing' && recordingStatus.state !== 'stopping') {
    return recordingStatus;
  }

  const activeFile = recordingStatus.activeFile;
  setRecordingStatus({ state: 'stopping', activeFile, lastError: undefined });

  try {
    const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'stopRecording' });
    if (res.type === 'stopRecordingResult' && res.ok) {
      setRecordingStatus({ state: 'idle', activeFile: undefined, lastError: undefined });
    } else {
      const message = res.type === 'stopRecordingResult' ? (res.error || 'Failed to stop recording') : 'Unexpected worker response for recording stop';
      throw new Error(message);
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Failed to stop recording';
    setRecordingStatus({ state: 'error', activeFile, lastError: message });
    throw error instanceof Error ? error : new Error(message);
  }
  return recordingStatus;
});

ipcMain.handle('suno:get-config', () => {
  return store.get('suno');
});

ipcMain.handle('suno:update-config', async (_event, config: SunoConfig) => {
  const sanitized: SunoConfig = { cookie: (config?.cookie ?? '').trim() };
  const previous = store.get('suno');
  store.set('suno', sanitized);

  try {
    if (sanitized.cookie !== previous.cookie) {
      await configureLibraryManager(sanitized.cookie);
    } else if (libraryManager) {
      await libraryManager.reload();
    } else {
      await configureLibraryManager(sanitized.cookie);
    }
  } catch (error) {
    console.error('Failed to apply Suno config:', error);
    throw error;
  }

  return store.get('suno');
});

let nativeUIPollingTimer: ReturnType<typeof setInterval> | null = null;

const startNativeUIPolling = () => {
  if (nativeUIPollingTimer) return;
  nativeUIPollingTimer = setInterval(() => {
    const native = getNativeUI();
    if (!native?.pollActions || !audioWorker) return;
    const actions = native.pollActions();
    for (const a of actions) {
      const deck = a.deck;
      switch (a.action) {
        case 'play':
          audioWorker.postMessage({ type: 'startDeck', deck });
          break;
        case 'stop':
          audioWorker.postMessage({ type: 'stop', deck });
          break;
        case 'crossfader':
          audioWorker.postMessage({ type: 'setCrossfader', position: a.value });
          break;
        case 'master_tempo':
          audioWorker.postMessage({ type: 'setMasterTempo', bpm: a.value });
          break;
        case 'cue': {
          audioWorker.postMessage({ type: 'setDeckCue', deck, enabled: a.value > 0.5 });
          break;
        }
        case 'eq': {
          audioWorker.postMessage({ type: 'setEqCut', deck, band: a.param, enabled: a.value > 0.5 });
          break;
        }
        case 'loop': {
          if (a.value <= 0) {
            audioWorker.postMessage({ type: 'clearLoop', deck });
          } else {
            const posFrames = deck === 1 ? cachedDeckAPosition : cachedDeckBPosition;
            const currentPosition = posFrames / cachedSampleRate;
            const trackId = deck === 1 ? deckATrackId : deckBTrackId;
            const beatGrid = trackId ? trackStructureMap.get(trackId)?.beats : undefined;
            audioWorker.postMessage({ type: 'setBeatLoop', deck, beats: a.value, masterTempo: cachedMasterTempo, currentPosition, beatGrid });
          }
          break;
        }
        case 'seek':
          audioWorker.postMessage({ type: 'seek', deck, position: a.value });
          break;
        case 'deck_gain':
          audioWorker.postMessage({ type: 'setDeckGain', deck, gain: a.value });
          break;
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

ipcMain.handle('native-ui:attach', async (_event, frame: NativeUIFrame) => {
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

ipcMain.handle('native-ui:set-frame', async (_event, frame: NativeUIFrame) => {
  const next = sanitizeNativeUIFrame(frame);
  if (next.width <= 0 || next.height <= 0) {
    return false;
  }
  return withNativeUI((native) => {
    native.setFrame(next.x, next.y, next.width, next.height);
    return true;
  }, false);
});

ipcMain.handle('native-ui:set-waveform', async (_event, deck: 1 | 2, samples: number[]) => {
  return withNativeUI((native) => {
    native.setWaveform(deck, samples);
    return true;
  }, false);
});

ipcMain.handle('native-ui:set-progress', async (_event, deck: 1 | 2, positionFrames: number, totalFrames: number, audioSampleRate: number) => {
  return withNativeUI((native) => {
    if (typeof native.setDeckProgress === 'function') {
      native.setDeckProgress(deck, positionFrames, totalFrames, audioSampleRate);
    }
    return true;
  }, false);
});

ipcMain.handle('native-ui:set-markers', async (_event, deck: 1 | 2, beats: number[], intro: number | null, outro: number | null) => {
  return withNativeUI((native) => {
    if (typeof native.setDeckMarkers === 'function') {
      native.setDeckMarkers(deck, beats, intro, outro);
    }
    return true;
  }, false);
});

ipcMain.handle('native-ui:set-console-state', async (_event, state: NativeConsoleState) => {
  return withNativeUI((native) => {
    if (typeof native.setConsoleState === 'function') {
      native.setConsoleState(state);
    }
    return true;
  }, false);
});

ipcMain.handle('native-ui:detach', async () => {
  return withNativeUI((native) => {
    stopNativeUIPolling();
    native.detach();
    return true;
  }, false);
});

ipcMain.handle('native-ui:set-artwork', async (_event, deck: 1 | 2, width: number, height: number, rgba: Buffer) => {
  return withNativeUI((native) => {
    if (typeof native.setDeckArtwork === 'function') {
      native.setDeckArtwork(deck, width, height, rgba);
    }
    return true;
  }, false);
});

ipcMain.handle('native-ui:clear-artwork', async (_event, deck: 1 | 2) => {
  return withNativeUI((native) => {
    if (typeof native.clearDeckArtwork === 'function') {
      native.clearDeckArtwork(deck);
    }
    return true;
  }, false);
});

ipcMain.handle('library:set-workspace', async (_event, workspace) => {
  await requireLibraryManager().setWorkspace(workspace);
});

ipcMain.handle('library:set-liked-filter', async (_event, enabled) => {
  await requireLibraryManager().setLikedFilter(enabled);
});

ipcMain.handle('library:toggle-liked-filter', async () => {
  await requireLibraryManager().toggleLikedFilter();
});

ipcMain.handle('library:download-track', async (_event, audioInfo) => {
  return await requireLibraryManager().downloadTrack(audioInfo);
});

ipcMain.handle('library:get-state', () => {
  return libraryStateCache;
});

ipcMain.handle('library:get-download-progress', () => {
  return libraryManager ? Array.from(libraryManager.getDownloadProgress().entries()) : [];
});

// Prefetch a single track image (cache locally)
// (removed) image-specific IPCs are not needed; images are bundled in library state

ipcMain.on('show-track-context-menu', (event, track) => {
  const menu = Menu.buildFromTemplate([
    {
      label: 'Load to Deck 1',
      enabled: !deckAPlaying,
      click: () => {
        event.sender.send('track-load-deck', { track, deck: 1 });
      },
    },
    {
      label: 'Load to Deck 2',
      enabled: !deckBPlaying,
      click: () => {
        event.sender.send('track-load-deck', { track, deck: 2 });
      },
    },
  ]);

  menu.popup({ window: BrowserWindow.fromWebContents(event.sender) || undefined });
});

// System info
ipcMain.handle('system:get-info', () => {
  const metrics = app.getAppMetrics();
  
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
ipcMain.handle('osc:get-config', () => {
  return store.get('osc');
});

ipcMain.handle('osc:update-config', async (_event, config: OSCConfig) => {
  store.set('osc', config);
  await sendWorkerMessage<WorkerOutMsg>({ type: 'updateOSCConfig', config });
});


// App lifecycle
app.on('ready', async () => {
  // Start audio worker
  try {
    const candidate = path.join(__dirname, 'audio-worker.js');
    const fsSync = await import('node:fs');
    if (!fsSync.existsSync(candidate)) {
      throw new Error(`Built worker not found at ${candidate}`);
    }
    audioWorker = new NodeWorker(candidate);
    
    // Track saved structure IDs to avoid redundant saves
    const savedStructureIds = new Set<string>();
        
    // Forward worker events to renderer
    audioWorker.on('message', (m: WorkerOutMsg) => {
      if (m.type === 'stateChanged') {
        cachedAudioState = m.state;
        // Update deck playing states
        deckAPlaying = m.state.deckAPlaying ?? false;
        deckBPlaying = m.state.deckBPlaying ?? false;
        if (m.state.masterTempo != null) cachedMasterTempo = m.state.masterTempo;
        if (m.state.deckAPosition != null) cachedDeckAPosition = m.state.deckAPosition;
        if (m.state.deckBPosition != null) cachedDeckBPosition = m.state.deckBPosition;
        if (m.state.sampleRate != null) cachedSampleRate = m.state.sampleRate;
        if (m.state.deckA?.id) {
          deckATrackId = m.state.deckA.id;
        }
        if (m.state.deckB?.id) {
          deckBTrackId = m.state.deckB.id;
        }
        pushNativeUiState();
        
        // Update MCP controller cache
        if (mcpController) {
          mcpController.updateAudioState(m.state);
        }
      } else if (m.type === 'levelState') {
        cachedLevelState = m.state;
        pushNativeConsoleState();
      } else if (m.type === 'trackEnded') {
        sendToRenderer('track-ended');
      } else if (m.type === 'error') {
        sendToRenderer('notification', `Audio Error: ${m.error}`);
      } else if (m.type === 'recordingError') {
        const activeFile = recordingStatus.activeFile;
        setRecordingStatus({ state: 'error', activeFile, lastError: m.error });
      } else if (m.type === 'waveformChunk') {
        const buffer = nativeWaveformBuffers.get(m.trackId) ?? {
          totalChunks: m.totalChunks,
          compactChunks: new Array<number[] | undefined>(m.totalChunks),
        };
        if (buffer.totalChunks !== m.totalChunks || buffer.compactChunks.length !== m.totalChunks) {
          buffer.totalChunks = m.totalChunks;
          buffer.compactChunks = new Array<number[] | undefined>(m.totalChunks);
        }
        if (m.chunkIndex >= 0 && m.chunkIndex < buffer.totalChunks) {
          buffer.compactChunks[m.chunkIndex] = m.chunk;
        }
        nativeWaveformBuffers.set(m.trackId, buffer);
        sendToRenderer('waveform-chunk', { trackId: m.trackId, chunkIndex: m.chunkIndex, totalChunks: m.totalChunks, chunk: m.chunk });
      } else if (m.type === 'waveformComplete') {
        const buffer = nativeWaveformBuffers.get(m.trackId);
        if (buffer) {
          let deck: 1 | 2 | null = null;
          if (deckATrackId === m.trackId) {
            deck = 1;
          } else if (deckBTrackId === m.trackId) {
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
              native.setWaveform(deck, flat);
              return true;
            }, false);
            }
          }

          nativeWaveformBuffers.delete(m.trackId);
        }
        sendToRenderer('waveform-complete', { trackId: m.trackId, totalFrames: m.totalFrames });
      } else if (m.type === 'trackStructure') {
        trackStructureMap.set(m.trackId, m.structure);
        pushNativeDeckMarkers();
        sendToRenderer('track-structure', { trackId: m.trackId, deck: m.deck, structure: m.structure });
        // Save track structure to cache (only once per track)
        if (!savedStructureIds.has(m.trackId)) {
          savedStructureIds.add(m.trackId);
          libraryManager?.saveTrackStructure(m.trackId, m.structure).catch((err) => {
            console.error('Failed to save track structure:', err);
          });
        }
      }
    });
    audioWorker.on('error', (err: unknown) => console.error('[AudioWorker] error', err));
    audioWorker.on('exit', () => {
      audioWorker = null;
    });
  } catch (err) {
    console.error('[AudioWorker] failed to start:', err);
    throw err; // Fail fast if worker cannot start
  }

  await initializeCore();
  
  // Start MCP server
  try {
    await startMcpServer(mcpController);
    console.log('MCP server started successfully');
  } catch (err) {
    console.error('[MCP] Failed to start server:', err);
    // Non-fatal, continue without MCP
  }
  
  createWindow();
});

app.on('window-all-closed', async () => {
  // Stop MCP server
  try {
    await stopMcpServer();
  } catch (err) {
    console.error('[MCP] Failed to stop server:', err);
  }

  if (audioWorker) {
    await sendWorkerMessage<WorkerOutMsg>({ type: 'cleanup' }).catch(() => {
      // Worker cleanup failed, continue shutdown
    });
    audioWorker.terminate();
    audioWorker = null;
  }

  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow();
  }
});

// Ensure recording is finalized on explicit app quit (e.g., Cmd+Q on macOS)
app.on('before-quit', async () => {
  try {
    if (recordingStatus.state === 'recording' || recordingStatus.state === 'preparing' || recordingStatus.state === 'stopping') {
      const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'stopRecording' });
      if (res.type === 'stopRecordingResult' && res.ok) {
        setRecordingStatus({ state: 'idle', activeFile: undefined, lastError: undefined });
      }
    }
  } catch (err) {
    console.error('[Recording] Failed to stop during before-quit:', err);
  }
});

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and import them here.
