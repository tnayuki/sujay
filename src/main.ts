import { app, BrowserWindow, ipcMain, Menu } from 'electron';
import path from 'node:path';
import { Worker as NodeWorker } from 'node:worker_threads';
import os from 'node:os';
import started from 'electron-squirrel-startup';
import Store from 'electron-store';

import { LibraryManager } from './core/library-manager.js';
import type { LibraryState, OSCConfig, AudioConfig } from './types.js';
import type { WorkerInMsg, WorkerOutMsg } from './workers/audio-worker-types.js';

declare const MAIN_WINDOW_VITE_DEV_SERVER_URL: string | undefined;
declare const MAIN_WINDOW_VITE_NAME: string;

// Handle creating/removing shortcuts on Windows when installing/uninstalling.
if (started) {
  app.quit();
}

// Check for SUNO_COOKIE environment variable
const SUNO_COOKIE = process.env.SUNO_COOKIE;

if (!SUNO_COOKIE) {
  console.error('Error: SUNO_COOKIE environment variable is not set');
  app.quit();
}

const sunoCacheDir = path.join(app.getPath('cache' as any), app.getName(), 'Suno');

// Initialize electron-store with schema
const store = new Store<{ osc: OSCConfig; audio: AudioConfig }>({
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
        deviceId: { type: ['number', 'null'] },
        mainChannels: { type: 'array', items: { type: ['number', 'null'] }, minItems: 2, maxItems: 2 },
        cueChannels: { type: 'array', items: { type: ['number', 'null'] }, minItems: 2, maxItems: 2 },
      },
      required: ['mainChannels', 'cueChannels'],
    },
  },
});

// Core modules
let libraryManager: LibraryManager;
let mainWindow: BrowserWindow | null = null;
let audioWorker: NodeWorker | null = null;

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

  // Create application menu
  const isMac = process.platform === 'darwin';
  const template: any[] = [
    ...(isMac
      ? [
          {
            label: app.name,
            submenu: [
              {
                label: 'Preferences...',
                accelerator: 'CmdOrCtrl+,',
                click: () => {
                  mainWindow?.webContents.send('open-preferences');
                },
              },
              { type: 'separator' },
              { role: 'hide' },
              { role: 'hideOthers' },
              { role: 'unhide' },
              { type: 'separator' },
              { role: 'quit' },
            ],
          },
        ]
      : []),
    {
      label: 'Edit',
      submenu: [
        { role: 'undo' },
        { role: 'redo' },
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        ...(isMac
          ? [
              { role: 'pasteAndMatchStyle' },
              { role: 'delete' },
              { role: 'selectAll' },
            ]
          : [{ role: 'delete' }, { type: 'separator' }, { role: 'selectAll' }]),
      ],
    },
    {
      label: 'View',
      submenu: [
        { role: 'reload' },
        { role: 'forceReload' },
        { role: 'toggleDevTools' },
        { type: 'separator' },
        { role: 'resetZoom' },
        { role: 'zoomIn' },
        { role: 'zoomOut' },
        { type: 'separator' },
        { role: 'togglefullscreen' },
      ],
    },
  ];

  const menu = Menu.buildFromTemplate(template);
  Menu.setApplicationMenu(menu);

  // Load the index.html of the app
  if (MAIN_WINDOW_VITE_DEV_SERVER_URL) {
    mainWindow.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL);
  } else {
    mainWindow.loadFile(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`));
  }

  // Open DevTools in development
  if (process.env.NODE_ENV === 'development') {
    mainWindow.webContents.openDevTools();
  }
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
      if ((outMsg as any).id === id) {
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

// Initialize core modules
async function initializeCore() {
  if (audioWorker) {
    // Initialize via worker
    const audioConfig = (store as any).get('audio');
    const oscConfig = (store as any).get('osc') as OSCConfig;
    const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'init', audioConfig, oscConfig });
    if (res.type === 'initResult' && !res.ok) {
      throw new Error(`Worker init failed: ${res.error}`);
    }
  }

  libraryManager = new LibraryManager(sunoCacheDir, SUNO_COOKIE!);
  
  await libraryManager.initialize();

  // Helper function to safely send to renderer
  const sendToRenderer = (channel: string, ...args: any[]) => {
    if (mainWindow && !mainWindow.isDestroyed() && mainWindow.webContents && !mainWindow.webContents.isDestroyed()) {
      try {
        // Check if webContents is still valid before sending
        if (mainWindow.webContents.getURL()) {
          mainWindow.webContents.send(channel, ...args);
        }
      } catch (error) {
        // Silently ignore "Render frame was disposed" errors (normal during reload/close)
        if (error instanceof Error && !error.message.includes('Render frame was disposed')) {
          console.error(`Error sending to renderer (${channel}):`, error);
        }
      }
    }
  };

  // Library events
  libraryManager.on('state-changed', (state: LibraryState) => {
    sendToRenderer('library-state-changed', state);
    sendToRenderer('download-progress-changed', libraryManager.getDownloadProgress());
  });

  libraryManager.on('sync-started', (data: any) => {
    sendToRenderer('library-sync-started', data);
  });

  libraryManager.on('sync-progress', (data: any) => {
    sendToRenderer('library-sync-progress', data);
  });

  libraryManager.on('sync-completed', (data: any) => {
    sendToRenderer('library-sync-completed', data);
  });

  libraryManager.on('sync-failed', (data: any) => {
    sendToRenderer('library-sync-failed', data);
  });


  // Download events
  libraryManager.on('download-started', (data: any) => {
    sendToRenderer('download-started', data);
  });

  libraryManager.on('download-progress', (data: any) => {
    sendToRenderer('download-progress', data);
  });

  libraryManager.on('download-completed', (data: any) => {
    sendToRenderer('download-completed', data);
  });

  libraryManager.on('download-failed', (data: any) => {
    sendToRenderer('download-failed', data);
  });

  // Library error events
  libraryManager.on('error', (error: Error) => {
    sendToRenderer('notification', `Library Error: ${error.message}`);
  });
}

// IPC Handlers
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

ipcMain.handle('audio:start-deck', async (_event, deck) => {
  await sendWorkerMessage<WorkerOutMsg>({ type: 'startDeck', deck });
});

// Audio device/config handlers
ipcMain.handle('audio:get-devices', () => {
  return new Promise((resolve, reject) => {
    const id = Date.now();
    const handler = (msg: any) => {
      if (msg && msg.type === 'devices' && msg.id === id) {
        audioWorker?.off('message', handler);
        resolve(msg.devices);
      }
    };
    audioWorker!.on('message', handler);
    try {
      audioWorker!.postMessage({ type: 'getDevices', id });
    } catch (e) {
      audioWorker!.off('message', handler);
      reject(e);
    }
    setTimeout(() => {
      audioWorker?.off('message', handler);
      reject(new Error('audio worker getDevices timeout'));
    }, 3000);
  });
});

ipcMain.handle('audio:get-config', () => {
  return (store as any).get('audio');
});

ipcMain.handle('audio:update-config', async (_event, config: AudioConfig) => {
  (store as any).set('audio', config);
  const res = await sendWorkerMessage<WorkerOutMsg>({ type: 'applyAudioConfig', config });
  if (res.type === 'applyAudioConfigResult' && !res.ok) {
    console.error('Failed to apply audio config in worker:', res.error);
  }
});

ipcMain.handle('library:set-workspace', async (_event, workspace) => {
  await libraryManager.setWorkspace(workspace);
});

ipcMain.handle('library:set-liked-filter', async (_event, enabled) => {
  await libraryManager.setLikedFilter(enabled);
});

ipcMain.handle('library:toggle-liked-filter', async () => {
  await libraryManager.toggleLikedFilter();
});

ipcMain.handle('library:download-track', async (_event, audioInfo) => {
  return await libraryManager.downloadTrack(audioInfo);
});

ipcMain.handle('library:get-state', () => {
  return libraryManager.getState();
});

ipcMain.handle('library:get-download-progress', () => {
  return Array.from(libraryManager.getDownloadProgress().entries());
});

// Prefetch a single track image (cache locally)
// (removed) image-specific IPCs are not needed; images are bundled in library state

ipcMain.on('show-track-context-menu', (event, track) => {
  const menu = Menu.buildFromTemplate([
    {
      label: 'Load to Deck 1',
      click: () => {
        event.sender.send('track-load-deck', { track, deck: 1 });
      },
    },
    {
      label: 'Load to Deck 2',
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
  
  return { time, cpuUsage: Math.round(totalCpuPercent * 10) / 10 };
});

// OSC config handlers
ipcMain.handle('osc:get-config', () => {
  const oscConfig = (store as any).get('osc') as OSCConfig;
  return oscConfig;
});

ipcMain.handle('osc:update-config', async (_event, config: OSCConfig) => {
  (store as any).set('osc', config);
  await sendWorkerMessage<WorkerOutMsg>({ type: 'updateOSCConfig', config });
});


// App lifecycle
app.on('ready', async () => {
  // Start audio worker
  try {
    const candidate = path.join(__dirname, 'audio-worker.js');
    const fs = await import('node:fs');
    if (!fs.existsSync(candidate)) {
      throw new Error(`Built worker not found at ${candidate}`);
    }
    audioWorker = new NodeWorker(candidate);
        
        // Helper to safely send to renderer
        const sendToRenderer = (channel: string, ...args: any[]) => {
          if (mainWindow && !mainWindow.isDestroyed() && mainWindow.webContents && !mainWindow.webContents.isDestroyed()) {
            try {
              if (mainWindow.webContents.getURL()) {
                mainWindow.webContents.send(channel, ...args);
              }
            } catch (error) {
              if (error instanceof Error && !error.message.includes('Render frame was disposed')) {
                console.error(`Error sending to renderer (${channel}):`, error);
              }
            }
          }
        };

    // Forward worker events to renderer
    audioWorker.on('message', (m: WorkerOutMsg) => {
      if (m.type === 'stateChanged') {
        sendToRenderer('audio-state-changed', m.state);
      } else if (m.type === 'trackEnded') {
        sendToRenderer('track-ended');
      } else if (m.type === 'error') {
        sendToRenderer('notification', `Audio Error: ${m.error}`);
      } else if (m.type === 'waveformChunk') {
        sendToRenderer('waveform-chunk', { trackId: m.trackId, chunkIndex: m.chunkIndex, totalChunks: m.totalChunks, chunk: m.chunk });
      } else if (m.type === 'waveformComplete') {
        sendToRenderer('waveform-complete', { trackId: m.trackId, totalFrames: m.totalFrames });
      }
    });
    audioWorker.on('error', (err: unknown) => console.error('[AudioWorker] error', err));
    audioWorker.on('exit', (code: number) => {
      audioWorker = null;
    });
  } catch (err) {
    console.error('[AudioWorker] failed to start:', err);
    throw err; // Fail fast if worker cannot start
  }

  await initializeCore();
  createWindow();
});

app.on('window-all-closed', async () => {
  if (audioWorker) {
    await sendWorkerMessage<WorkerOutMsg>({ type: 'cleanup' }).catch(() => {});
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

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and import them here.
