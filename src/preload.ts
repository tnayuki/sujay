/**
 * Electron Preload Script
 * Exposes secure IPC bridge to renderer process
 */

import { contextBridge, ipcRenderer } from 'electron';
import type { AudioEngineState, LibraryState, Track, Workspace } from './types';

// Expose protected methods that allow the renderer process to use
// the ipcRenderer without exposing the entire object
contextBridge.exposeInMainWorld('electronAPI', {
  // Audio Engine
  audioPlay: (track: Track) => ipcRenderer.invoke('audio:play', track),
  audioStop: () => ipcRenderer.invoke('audio:stop'),
  audioGetState: () => ipcRenderer.invoke('audio:get-state'),
  audioSeek: (position: number) => ipcRenderer.invoke('audio:seek', position),

  // Library Manager
  librarySetWorkspace: (workspace: Workspace | null) => ipcRenderer.invoke('library:set-workspace', workspace),
  librarySetLikedFilter: (enabled: boolean) => ipcRenderer.invoke('library:set-liked-filter', enabled),
  libraryToggleLikedFilter: () => ipcRenderer.invoke('library:toggle-liked-filter'),
  libraryDownloadTrack: (audioInfo: any) => ipcRenderer.invoke('library:download-track', audioInfo),
  libraryGetState: () => ipcRenderer.invoke('library:get-state'),
  libraryGetDownloadProgress: () => ipcRenderer.invoke('library:get-download-progress'),

  // Event listeners - return cleanup functions
  onAudioStateChanged: (callback: (state: AudioEngineState) => void) => {
    const listener = (_event: any, state: AudioEngineState) => callback(state);
    ipcRenderer.on('audio-state-changed', listener);
    return () => ipcRenderer.removeListener('audio-state-changed', listener);
  },
  onLibraryStateChanged: (callback: (state: LibraryState) => void) => {
    const listener = (_event: any, state: LibraryState) => callback(state);
    ipcRenderer.on('library-state-changed', listener);
    return () => ipcRenderer.removeListener('library-state-changed', listener);
  },
  onDownloadProgressChanged: (callback: (progress: Map<string, string>) => void) => {
    const listener = (_event: any, progress: [string, string][]) => callback(new Map(progress));
    ipcRenderer.on('download-progress-changed', listener);
    return () => ipcRenderer.removeListener('download-progress-changed', listener);
  },
  onNotification: (callback: (message: string) => void) => {
    const listener = (_event: any, message: string) => callback(message);
    ipcRenderer.on('notification', listener);
    return () => ipcRenderer.removeListener('notification', listener);
  },

  // Sync events
  onLibrarySyncStarted: (callback: (data: any) => void) => {
    const listener = (_event: any, data: any) => callback(data);
    ipcRenderer.on('library-sync-started', listener);
    return () => ipcRenderer.removeListener('library-sync-started', listener);
  },
  onLibrarySyncProgress: (callback: (data: any) => void) => {
    const listener = (_event: any, data: any) => callback(data);
    ipcRenderer.on('library-sync-progress', listener);
    return () => ipcRenderer.removeListener('library-sync-progress', listener);
  },
  onLibrarySyncCompleted: (callback: (data: any) => void) => {
    const listener = (_event: any, data: any) => callback(data);
    ipcRenderer.on('library-sync-completed', listener);
    return () => ipcRenderer.removeListener('library-sync-completed', listener);
  },
  onLibrarySyncFailed: (callback: (data: any) => void) => {
    const listener = (_event: any, data: any) => callback(data);
    ipcRenderer.on('library-sync-failed', listener);
    return () => ipcRenderer.removeListener('library-sync-failed', listener);
  },

});

// Type declaration for window.electronAPI
export interface ElectronAPI {
  audioPlay: (track: Track) => Promise<void>;
  audioStop: () => Promise<void>;
  audioGetState: () => Promise<AudioEngineState>;
  audioSeek: (position: number) => Promise<void>;
  librarySetWorkspace: (workspace: Workspace | null) => Promise<void>;
  librarySetLikedFilter: (enabled: boolean) => Promise<void>;
  libraryToggleLikedFilter: () => Promise<void>;
  libraryDownloadTrack: (audioInfo: any) => Promise<Track>;
  libraryGetState: () => Promise<LibraryState>;
  libraryGetDownloadProgress: () => Promise<[string, string][]>;
  onAudioStateChanged: (callback: (state: AudioEngineState) => void) => void;
  onLibraryStateChanged: (callback: (state: LibraryState) => void) => void;
  onDownloadProgressChanged: (callback: (progress: Map<string, string>) => void) => void;
  onNotification: (callback: (message: string) => void) => void;
  onLibrarySyncStarted: (callback: (data: any) => void) => void;
  onLibrarySyncProgress: (callback: (data: any) => void) => void;
  onLibrarySyncCompleted: (callback: (data: any) => void) => void;
  onLibrarySyncFailed: (callback: (data: any) => void) => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
