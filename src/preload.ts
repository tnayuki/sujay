/**
 * Electron Preload Script
 * Exposes secure IPC bridge to renderer process
 */

import { contextBridge, ipcRenderer } from 'electron';
import type { AudioEngineState, LibraryState, Track, Workspace, OSCConfig } from './types';

// Expose protected methods that allow the renderer process to use
// the ipcRenderer without exposing the entire object
contextBridge.exposeInMainWorld('electronAPI', {
  // Audio Engine
  audioPlay: (track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null) => ipcRenderer.invoke('audio:play', track, crossfade, targetDeck ?? null),
  audioStop: (deck: 1 | 2) => ipcRenderer.invoke('audio:stop', deck),
  audioGetState: () => ipcRenderer.invoke('audio:get-state'),
  audioSeek: (deck: 1 | 2, position: number) => ipcRenderer.invoke('audio:seek', deck, position),
  audioSetCrossfader: (position: number) => ipcRenderer.invoke('audio:set-crossfader', position),
  audioSetMasterTempo: (bpm: number) => ipcRenderer.invoke('audio:set-master-tempo', bpm),
  audioStartDeck: (deck: 1 | 2) => ipcRenderer.invoke('audio:start-deck', deck),

  // Library Manager
  librarySetWorkspace: (workspace: Workspace | null) => ipcRenderer.invoke('library:set-workspace', workspace),
  librarySetLikedFilter: (enabled: boolean) => ipcRenderer.invoke('library:set-liked-filter', enabled),
  libraryToggleLikedFilter: () => ipcRenderer.invoke('library:toggle-liked-filter'),
  libraryDownloadTrack: (audioInfo: any) => ipcRenderer.invoke('library:download-track', audioInfo),
  libraryGetState: () => ipcRenderer.invoke('library:get-state'),
  libraryGetDownloadProgress: () => ipcRenderer.invoke('library:get-download-progress'),

  showTrackContextMenu: (track: any) => ipcRenderer.send('show-track-context-menu', track),

  // System info
  getSystemInfo: () => ipcRenderer.invoke('system:get-info'),

  // OSC Config
  oscGetConfig: () => ipcRenderer.invoke('osc:get-config'),
  oscUpdateConfig: (config: OSCConfig) => ipcRenderer.invoke('osc:update-config', config),

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

  onTrackLoadDeck: (callback: (data: { track: any; deck: 1 | 2 }) => void) => {
    const listener = (_event: any, data: { track: any; deck: 1 | 2 }) => callback(data);
    ipcRenderer.on('track-load-deck', listener);
    return () => ipcRenderer.removeListener('track-load-deck', listener);
  },

  onWaveformChunk: (callback: (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => void) => {
    const listener = (_event: any, data: any) => callback(data);
    ipcRenderer.on('waveform-chunk', listener);
    return () => ipcRenderer.removeListener('waveform-chunk', listener);
  },

  onWaveformComplete: (callback: (data: { trackId: string; totalFrames: number }) => void) => {
    const listener = (_event: any, data: any) => callback(data);
    ipcRenderer.on('waveform-complete', listener);
    return () => ipcRenderer.removeListener('waveform-complete', listener);
  },

  onOpenPreferences: (callback: () => void) => {
    const listener = () => callback();
    ipcRenderer.on('open-preferences', listener);
    return () => ipcRenderer.removeListener('open-preferences', listener);
  },

});

// Type declaration for window.electronAPI
export interface ElectronAPI {
  audioPlay: (track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null) => Promise<void>;
  audioStop: (deck: 1 | 2) => Promise<void>;
  audioGetState: () => Promise<AudioEngineState>;
  audioSeek: (deck: 1 | 2, position: number) => Promise<void>;
  audioSetCrossfader: (position: number) => Promise<void>;
  audioSetMasterTempo: (bpm: number) => Promise<void>;
  audioStartDeck: (deck: 1 | 2) => Promise<void>;
  librarySetWorkspace: (workspace: Workspace | null) => Promise<void>;
  librarySetLikedFilter: (enabled: boolean) => Promise<void>;
  libraryToggleLikedFilter: () => Promise<void>;
  libraryDownloadTrack: (audioInfo: any) => Promise<Track>;
  libraryGetState: () => Promise<LibraryState>;
  libraryGetDownloadProgress: () => Promise<[string, string][]>;
  showTrackContextMenu: (track: any) => void;
  getSystemInfo: () => Promise<{ time: string; cpuUsage: number }>;
  onAudioStateChanged: (callback: (state: AudioEngineState) => void) => () => void;
  onLibraryStateChanged: (callback: (state: LibraryState) => void) => () => void;
  onDownloadProgressChanged: (callback: (progress: Map<string, string>) => void) => () => void;
  onNotification: (callback: (message: string) => void) => () => void;
  onLibrarySyncStarted: (callback: (data: any) => void) => () => void;
  onLibrarySyncProgress: (callback: (data: any) => void) => () => void;
  onLibrarySyncCompleted: (callback: (data: any) => void) => () => void;
  onLibrarySyncFailed: (callback: (data: any) => void) => () => void;
  onTrackLoadDeck: (callback: (data: { track: any; deck: 1 | 2 }) => void) => () => void;
  onWaveformChunk: (callback: (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => void) => () => void;
  onWaveformComplete: (callback: (data: { trackId: string; totalFrames: number }) => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
