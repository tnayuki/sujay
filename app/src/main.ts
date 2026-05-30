/**
 * Electron main process — thin host glue for the Electron-based launch path.
 *
 * Business logic lives in:
 *   app/src/main/runtime/audio-runtime.ts   ← Electron-free audio core
 *   app/src/main/settings/app-settings-store.ts
 *   app/src/main/host/shell-host.ts
 *   app/src/main/host/electron-ipc-host.ts
 *   app/src/main/runtime/lifecycle-bootstrap.ts
 *
 * [LEGACY-ELECTRON] markers indicate code that exists only because the
 * current host is Electron.  Each marker is a candidate for removal once
 * the Rust-native host path is complete (issue #29).
 */

import { app, BrowserWindow, ipcMain, Menu } from 'electron';
import path from 'node:path';
import started from 'electron-squirrel-startup';

import { IPC_CHANNELS, IPC_EVENTS } from './main/ipc-contract';
import { createElectronIpcHost } from './main/host/electron-ipc-host';
import { createElectronShellHost } from './main/host/shell-host';
import { createAppSettingsStore } from './main/settings/app-settings-store';
import { registerRuntimeLifecycle } from './main/runtime/lifecycle-bootstrap';
import { createAudioRuntime } from './main/runtime/audio-runtime';

import type {
  AudioConfig,
  AudioEngineState,
  OSCConfig,
  RecordingConfig,
  Track,
} from './types';

// ---------------------------------------------------------------------------
// Rust native-UI types  [LEGACY-ELECTRON] — these go away when the Rust host
// owns the window; the native module will be called directly from Rust.
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Vite injected constants  [LEGACY-ELECTRON]
// ---------------------------------------------------------------------------

declare const MAIN_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const MAIN_WINDOW_VITE_NAME: string;
declare const PREFERENCES_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const PREFERENCES_WINDOW_VITE_NAME: string;

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

// [LEGACY-ELECTRON] Squirrel installer shortcut handling
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
const settingsStore = createAppSettingsStore(
  path.join(shellHost.getPath('userData'), 'settings.json'),
  defaultRecordingConfig,
);

// [LEGACY-ELECTRON] IPC host — replaced by direct function calls in Rust host
const ipcHost = createElectronIpcHost({
  ipcMain,
  getMainWindow: () => mainWindow,
});

// ---------------------------------------------------------------------------
// Audio runtime (Electron-free core)
// ---------------------------------------------------------------------------

const audioRuntime = createAudioRuntime(settingsStore, {
  onStateUpdate() {
    pushNativeUiState();
  },
  onWaveformChunk(data) {
    ipcHost.send(IPC_EVENTS.waveformChunk, data);
  },
  onWaveformComplete(data) {
    ipcHost.send(IPC_EVENTS.waveformComplete, data);
  },
  onNativeWaveformReady(deck, samples) {
    withNativeUI((native) => {
      native.setWaveform(deck, samples);
      return true;
    }, false);
  },
  onTrackStructure(data) {
    ipcHost.send(IPC_EVENTS.trackStructure, data);
    pushNativeDeckMarkers();
  },
  onRecordingStatus(status) {
    ipcHost.send(IPC_EVENTS.recordingStatus, status);
  },
});

async function initializeCore() {
  await audioRuntime.initialize();
}

// ---------------------------------------------------------------------------
// [LEGACY-ELECTRON] Native UI bridge — called from Electron main process until
// the Rust host owns this directly via the @sujay/ui addon API.
// ---------------------------------------------------------------------------

let nativeUI: NativeUIModule | null = null;

const getNativeUI = (): NativeUIModule | null => {
  if (process.platform !== 'darwin') return null;
  if (nativeUI) return nativeUI;
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    nativeUI = require('@sujay/ui') as NativeUIModule;
    return nativeUI;
  } catch (error) {
    console.error('[native-ui] failed to load module:', error);
    return null;
  }
};

const withNativeUI = <T>(fn: (native: NativeUIModule) => T, fallback: T): T => {
  const native = getNativeUI();
  if (!native) return fallback;
  try {
    return fn(native);
  } catch (error) {
    console.error('[native-ui] operation failed:', error);
    return fallback;
  }
};

const sanitizeNativeUIFrame = (frame: NativeUIFrame): NativeUIFrame => ({
  x: Math.max(0, Math.round(frame.x)),
  y: Math.max(0, Math.round(frame.y)),
  width: Math.max(0, Math.round(frame.width)),
  height: Math.max(0, Math.round(frame.height)),
});

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
  const s = audioRuntime.getCachedState();
  const ls = audioRuntime.getCachedLevelState();
  const sampleRate = audioRuntime.getCachedSampleRate();
  const consoleState: NativeConsoleState = {
    deckA: buildNativeDeckConsoleState(
      s.deckA, s.deckAPosition ?? 0, s.deckAPlaying,
      s.deckALoop, s.deckACueEnabled, s.deckAEqCut,
      s.deckAGain ?? 1.0, ls.deckAPeak, sampleRate,
    ),
    deckB: buildNativeDeckConsoleState(
      s.deckB, s.deckBPosition ?? 0, s.deckBPlaying,
      s.deckBLoop, s.deckBCueEnabled, s.deckBEqCut,
      s.deckBGain ?? 1.0, ls.deckBPeak, sampleRate,
    ),
    masterTempo: s.masterTempo ?? audioRuntime.getCachedMasterTempo(),
    crossfader: s.crossfaderPosition ?? 0.5,
  };
  withNativeUI((native) => {
    if (typeof native.setConsoleState === 'function') native.setConsoleState(consoleState);
    return true;
  }, false);
};

const pushNativeDeckProgress = () => {
  const s = audioRuntime.getCachedState();
  const sr = audioRuntime.getCachedSampleRate();
  withNativeUI((native) => {
    if (typeof native.setDeckProgress === 'function') {
      native.setDeckProgress(1, s.deckAPosition ?? 0, audioRuntime.getDeckATotalFrames(), sr);
      native.setDeckProgress(2, s.deckBPosition ?? 0, audioRuntime.getDeckBTotalFrames(), sr);
    }
    return true;
  }, false);
};

const pushNativeDeckMarkers = () => {
  const sr = audioRuntime.getCachedSampleRate();
  const deckStates = [
    { deck: 1 as const, trackId: audioRuntime.getDeckATrackId() },
    { deck: 2 as const, trackId: audioRuntime.getDeckBTrackId() },
  ];
  withNativeUI((native) => {
    if (typeof native.setDeckMarkers === 'function') {
      for (const { deck, trackId } of deckStates) {
        const structure = trackId ? audioRuntime.getTrackStructure(trackId) : undefined;
        const beats = (structure?.beats ?? []).map((b) => b * sr);
        const intro = structure?.intro?.end != null ? structure.intro.end * sr : null;
        const outro = structure?.outro?.start != null ? structure.outro.start * sr : null;
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

// ---------------------------------------------------------------------------
// [LEGACY-ELECTRON] Native UI polling loop
// Polls egui actions from native UI and forwards to audio runtime.
// In the Rust-native host this loop will be replaced by a native event handler.
// ---------------------------------------------------------------------------

let nativeUIPollingTimer: ReturnType<typeof setInterval> | null = null;

const startNativeUIPolling = () => {
  if (nativeUIPollingTimer) return;
  nativeUIPollingTimer = setInterval(() => {
    const native = getNativeUI();
    if (!native?.pollActions) return;
    const actions = native.pollActions();
    for (const a of actions) {
      const deck = a.deck as 1 | 2;
      switch (a.action) {
        case 'play': audioRuntime.startDeck(deck); break;
        case 'stop': audioRuntime.stop(deck); break;
        case 'crossfader': audioRuntime.setCrossfader(a.value); break;
        case 'master_tempo': audioRuntime.setMasterTempo(a.value); break;
        case 'cue': audioRuntime.setDeckCue(deck, a.value > 0.5); break;
        case 'eq': audioRuntime.setEqCut(deck, a.param as 'low' | 'mid' | 'high', a.value > 0.5); break;
        case 'loop': {
          if (a.value <= 0) {
            audioRuntime.clearLoop(deck);
          } else {
            const posFrames = deck === 1 ? audioRuntime.getDeckAPosition() : audioRuntime.getDeckBPosition();
            const currentPosition = posFrames / audioRuntime.getCachedSampleRate();
            const trackId = deck === 1 ? audioRuntime.getDeckATrackId() : audioRuntime.getDeckBTrackId();
            const beatGrid = trackId ? audioRuntime.getTrackStructure(trackId)?.beats : undefined;
            audioRuntime.setBeatLoop(deck, a.value, audioRuntime.getCachedMasterTempo(), currentPosition, beatGrid);
          }
          break;
        }
        case 'seek': audioRuntime.seek(deck, a.value); break;
        case 'deck_gain': audioRuntime.setDeckGain(deck, a.value); break;
        case 'load_file': {
          const filePath = a.param?.trim();
          if (!filePath) break;
          const base = path.basename(filePath);
          const title = base.replace(/\.[^.]+$/, '') || base;
          const track: Track = {
            id: `local:${filePath}:${Date.now()}`,
            title,
            mp3Path: filePath,
            duration: 0,
          };
          void audioRuntime.loadTrack(track, deck).catch((error) => {
            const message = error instanceof Error ? error.message : String(error);
            ipcHost.send(IPC_EVENTS.notification, `Audio Error: ${message}`);
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

// ---------------------------------------------------------------------------
// [LEGACY-ELECTRON] Window management
// ---------------------------------------------------------------------------

let mainWindow: BrowserWindow | null = null;
let preferencesWindow: BrowserWindow | null = null;

const createWindow = () => {
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

  const isMac = shellHost.platform === 'darwin';
  const template: Electron.MenuItemConstructorOptions[] = [];
  const SEP: Electron.MenuItemConstructorOptions = { type: 'separator' };

  if (isMac) {
    template.push({
      label: shellHost.appName,
      submenu: [
        { label: 'Preferences...', accelerator: 'CmdOrCtrl+,', click: () => createPreferencesWindow() },
        SEP,
        { role: 'hide' }, { role: 'hideOthers' }, { role: 'unhide' },
        SEP,
        { role: 'quit' },
      ],
    });
  }

  template.push({
    label: 'Edit',
    submenu: (() => {
      const editSub: Electron.MenuItemConstructorOptions[] = [
        { role: 'undo' }, { role: 'redo' }, SEP,
        { role: 'cut' }, { role: 'copy' }, { role: 'paste' },
      ];
      if (isMac) editSub.push({ role: 'pasteAndMatchStyle' }, { role: 'delete' }, { role: 'selectAll' });
      else editSub.push({ role: 'delete' }, SEP, { role: 'selectAll' });
      return editSub;
    })(),
  });

  template.push({
    label: 'View',
    submenu: [
      { role: 'reload' }, { role: 'forceReload' }, { role: 'toggleDevTools' }, SEP,
      { role: 'resetZoom' }, { role: 'zoomIn' }, { role: 'zoomOut' }, SEP,
      { role: 'togglefullscreen' },
    ],
  });

  shellHost.setApplicationMenu(shellHost.buildMenuFromTemplate(template));

  if (MAIN_WINDOW_VITE_DEV_SERVER_URL) {
    mainWindow.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL);
  } else {
    mainWindow.loadFile(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`));
  }
};

const createPreferencesWindow = () => {
  if (!mainWindow) return;
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

  preferencesWindow.on('closed', () => { preferencesWindow = null; });

  if (PREFERENCES_WINDOW_VITE_DEV_SERVER_URL) {
    preferencesWindow.loadURL(new URL('preferences.html', PREFERENCES_WINDOW_VITE_DEV_SERVER_URL).toString());
  } else {
    preferencesWindow.loadFile(path.join(__dirname, `../renderer/${PREFERENCES_WINDOW_VITE_NAME}/preferences.html`));
  }

  preferencesWindow.once('ready-to-show', () => { preferencesWindow?.show(); });
};

// ---------------------------------------------------------------------------
// IPC handlers — [LEGACY-ELECTRON] thin wrappers over AudioRuntime
// ---------------------------------------------------------------------------

// Audio
ipcHost.handle(IPC_CHANNELS.audio.loadTrack, async (track: Track, deck: 1 | 2) => {
  await initializeCore();
  await audioRuntime.loadTrack(track, deck);
});

ipcHost.handle(IPC_CHANNELS.audio.play, async (track: Track, crossfade: boolean, targetDeck: 1 | 2 | null) => {
  await initializeCore();
  await audioRuntime.play(track, crossfade, targetDeck);
});

ipcHost.handle(IPC_CHANNELS.audio.stop, async (deck: 1 | 2) => {
  await initializeCore();
  audioRuntime.stop(deck);
});

ipcHost.handle(IPC_CHANNELS.audio.getState, async () => {
  await initializeCore();
  return audioRuntime.getCachedState();
});

ipcHost.handle(IPC_CHANNELS.audio.seek, async (deck: 1 | 2, position: number) => {
  await initializeCore();
  audioRuntime.seek(deck, position);
});

ipcHost.handle(IPC_CHANNELS.audio.setCrossfader, async (position: number) => {
  await initializeCore();
  audioRuntime.setCrossfader(position);
});

ipcHost.handle(IPC_CHANNELS.audio.setMasterTempo, async (bpm: number) => {
  await initializeCore();
  audioRuntime.setMasterTempo(bpm);
});

ipcHost.handle(IPC_CHANNELS.audio.setDeckCue, async (deck: 1 | 2, enabled: boolean) => {
  await initializeCore();
  audioRuntime.setDeckCue(deck, enabled);
});

ipcHost.handle(IPC_CHANNELS.audio.setEqCut, async (deck: 1 | 2, band: string, enabled: boolean) => {
  await initializeCore();
  audioRuntime.setEqCut(deck, band as 'low' | 'mid' | 'high', enabled);
});

ipcHost.handle(IPC_CHANNELS.audio.setDeckGain, async (deck: 1 | 2, gain: number) => {
  await initializeCore();
  audioRuntime.setDeckGain(deck, gain);
});

ipcHost.handle(IPC_CHANNELS.audio.setBeatLoop, async (deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) => {
  await initializeCore();
  audioRuntime.setBeatLoop(deck, beats, masterTempo, currentPosition, beatGrid);
});

ipcHost.handle(IPC_CHANNELS.audio.clearLoop, async (deck: 1 | 2) => {
  await initializeCore();
  audioRuntime.clearLoop(deck);
});

ipcHost.handle(IPC_CHANNELS.audio.startDeck, async (deck: 1 | 2) => {
  await initializeCore();
  audioRuntime.startDeck(deck);
});

ipcHost.handle(IPC_CHANNELS.audio.setMicEnabled, async (enabled: boolean) => {
  await initializeCore();
  audioRuntime.setMicEnabled(enabled);
});

ipcHost.handle(IPC_CHANNELS.audio.getDevices, async () => audioRuntime.listDevices());

ipcHost.handle(IPC_CHANNELS.audio.getConfig, () => settingsStore.getAudioConfig());

ipcHost.handle(IPC_CHANNELS.audio.updateConfig, async (config: AudioConfig) => {
  settingsStore.setAudioConfig(config);
  await initializeCore();
  try {
    audioRuntime.applyAudioConfig(config);
  } catch (error) {
    console.error('[audio] failed to apply audio config:', error);
  }
});

// Recording
ipcHost.handle(IPC_CHANNELS.recording.getConfig, () => settingsStore.getRecordingConfig());
ipcHost.handle(IPC_CHANNELS.recording.updateConfig, (config: RecordingConfig) => {
  settingsStore.setRecordingConfig(config);
  return settingsStore.getRecordingConfig();
});
ipcHost.handle(IPC_CHANNELS.recording.getStatus, () => audioRuntime.getRecordingStatus());
ipcHost.handle(IPC_CHANNELS.recording.start, async (format: 'wav' | 'ogg') => audioRuntime.startRecording(format));
ipcHost.handle(IPC_CHANNELS.recording.stop, async () => audioRuntime.stopRecording());

// [LEGACY-ELECTRON] Native UI IPC handlers
ipcHost.handle(IPC_CHANNELS.nativeUi.attach, async (frame: NativeUIFrame) => {
  const win = mainWindow;
  if (!win || win.isDestroyed()) return false;
  const next = sanitizeNativeUIFrame(frame);
  if (next.width <= 0 || next.height <= 0) return false;
  return withNativeUI((native) => {
    native.attach(win.getNativeWindowHandle(), next.x, next.y, next.width, next.height);
    startNativeUIPolling();
    pushNativeUiState();
    return true;
  }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.setFrame, async (frame: NativeUIFrame) => {
  const next = sanitizeNativeUIFrame(frame);
  if (next.width <= 0 || next.height <= 0) return false;
  return withNativeUI((native) => { native.setFrame(next.x, next.y, next.width, next.height); return true; }, false);
});

ipcHost.handle(IPC_CHANNELS.nativeUi.setWaveform, async (deck: 1 | 2, samples: number[]) =>
  withNativeUI((native) => { native.setWaveform(deck, samples); return true; }, false));

ipcHost.handle(IPC_CHANNELS.nativeUi.detach, async () =>
  withNativeUI((native) => { stopNativeUIPolling(); native.detach(); return true; }, false));

ipcHost.handle(IPC_CHANNELS.nativeUi.setArtwork, async (deck: 1 | 2, width: number, height: number, rgba: Buffer) =>
  withNativeUI((native) => { native.setDeckArtwork?.(deck, width, height, rgba); return true; }, false));

ipcHost.handle(IPC_CHANNELS.nativeUi.clearArtwork, async (deck: 1 | 2) =>
  withNativeUI((native) => { native.clearDeckArtwork?.(deck); return true; }, false));

// System info  [LEGACY-ELECTRON]
ipcHost.handle(IPC_CHANNELS.system.getInfo, () => {
  const metrics = shellHost.getAppMetrics();
  const totalCpuPercent = metrics.reduce((sum, m) => sum + m.cpu.percentCPUUsage, 0);
  const time = new Date().toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  return { time, cpuUsage: Math.round(totalCpuPercent * 10) / 10, memoryUsage: Math.round(process.memoryUsage().rss / 1024 / 1024) };
});

// OSC config
ipcHost.handle(IPC_CHANNELS.osc.getConfig, () => settingsStore.getOscConfig());
ipcHost.handle(IPC_CHANNELS.osc.updateConfig, (config: OSCConfig) => {
  settingsStore.setOscConfig(config);
  audioRuntime.updateOscConfig(config);
});

// ---------------------------------------------------------------------------
// App lifecycle  [LEGACY-ELECTRON]
// ---------------------------------------------------------------------------

registerRuntimeLifecycle(shellHost, {
  onReady: async () => {
    await initializeCore();
    createWindow();
  },

  onWindowAllClosed: async () => {
    audioRuntime.close();
    if (shellHost.platform !== 'darwin') shellHost.quit();
  },

  onActivate: () => {
    if (shellHost.shouldOpenMainWindow()) createWindow();
  },

  onBeforeQuit: async () => {
    try {
      await audioRuntime.stopRecording();
    } catch {
      // best-effort
    }
  },
});
