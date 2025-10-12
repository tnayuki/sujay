import { app, BrowserWindow, ipcMain } from 'electron';
import path from 'node:path';
import started from 'electron-squirrel-startup';

import { AudioEngine } from './core/audio-engine.js';
import { LibraryManager } from './core/library-manager.js';
import type { AudioEngineState, LibraryState } from './types.js';

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

// Core modules
let audioEngine: AudioEngine;
let libraryManager: LibraryManager;
let mainWindow: BrowserWindow | null = null;

const createWindow = () => {
  // Create the browser window
  mainWindow = new BrowserWindow({
    width: 1200,
    height: 800,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
    },
  });

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

// Initialize core modules
async function initializeCore() {
  audioEngine = new AudioEngine();
  libraryManager = new LibraryManager(sunoCacheDir, SUNO_COOKIE!);

  await audioEngine.initialize();
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

  // Forward events to renderer
  audioEngine.on('state-changed', (state: AudioEngineState) => {
    sendToRenderer('audio-state-changed', state);
  });

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

  // Error events
  audioEngine.on('error', (error: Error) => {
    sendToRenderer('notification', `Audio Error: ${error.message}`);
  });

  libraryManager.on('error', (error: Error) => {
    sendToRenderer('notification', `Library Error: ${error.message}`);
  });
}

// IPC Handlers
ipcMain.handle('audio:play', async (_event, track) => {
  try {
    await audioEngine.play(track);
  } catch (error: any) {
    throw new Error(error.message);
  }
});

ipcMain.handle('audio:stop', () => {
  audioEngine.stop();
});

ipcMain.handle('audio:get-state', () => {
  return audioEngine.getState();
});

ipcMain.handle('audio:seek', (_event, position) => {
  audioEngine.seek(position);
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


// App lifecycle
app.on('ready', async () => {
  await initializeCore();
  createWindow();
});

app.on('window-all-closed', async () => {
  await audioEngine.cleanup();
  libraryManager.cleanup();

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
