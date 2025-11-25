/**
 * Electron Preload Script
 * Exposes secure IPC bridge to renderer process
 */

import { contextBridge, ipcRenderer } from 'electron';
import type {
  AudioEngineState,
  AudioLevelState,
  LibraryState,
  Track,
  Workspace,
  OSCConfig,
  AudioConfig,
  RecordingConfig,
  RecordingStatus,
} from './types';
import type { AudioInfo } from './suno-api';

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
  audioSetDeckCue: (deck: 1 | 2, enabled: boolean) => ipcRenderer.invoke('audio:set-deck-cue', deck, enabled),
  audioStartDeck: (deck: 1 | 2) => ipcRenderer.invoke('audio:start-deck', deck),
  audioSetTalkover: (pressed: boolean) => ipcRenderer.invoke('audio:set-talkover', pressed),
  
  // Audio Config
  audioGetDevices: () => ipcRenderer.invoke('audio:get-devices'),
  audioGetConfig: () => ipcRenderer.invoke('audio:get-config'),
  audioUpdateConfig: (config: AudioConfig) => ipcRenderer.invoke('audio:update-config', config),

  // Library Manager
  librarySetWorkspace: (workspace: Workspace | null) => ipcRenderer.invoke('library:set-workspace', workspace),
  librarySetLikedFilter: (enabled: boolean) => ipcRenderer.invoke('library:set-liked-filter', enabled),
  libraryToggleLikedFilter: () => ipcRenderer.invoke('library:toggle-liked-filter'),
  libraryDownloadTrack: (audioInfo: AudioInfo) => ipcRenderer.invoke('library:download-track', audioInfo),
  libraryGetState: () => ipcRenderer.invoke('library:get-state'),
  libraryGetDownloadProgress: () => ipcRenderer.invoke('library:get-download-progress'),

  showTrackContextMenu: (track: Track) => ipcRenderer.send('show-track-context-menu', track),

  // System info
  getSystemInfo: () => ipcRenderer.invoke('system:get-info'),

  // OSC Config
  oscGetConfig: () => ipcRenderer.invoke('osc:get-config'),
  oscUpdateConfig: (config: OSCConfig) => ipcRenderer.invoke('osc:update-config', config),

  // Recording
  recordingGetConfig: () => ipcRenderer.invoke('recording:get-config'),
  recordingUpdateConfig: (config: RecordingConfig) => ipcRenderer.invoke('recording:update-config', config),
  recordingGetStatus: () => ipcRenderer.invoke('recording:get-status'),
  recordingStart: () => ipcRenderer.invoke('recording:start'),
  recordingStop: () => ipcRenderer.invoke('recording:stop'),

  // Event listeners - return cleanup functions
  onAudioStateChanged: (callback: (state: AudioEngineState) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, state: AudioEngineState) => callback(state);
    ipcRenderer.on('audio-state-changed', listener);
    return () => ipcRenderer.removeListener('audio-state-changed', listener);
  },
  onAudioLevelState: (callback: (state: AudioLevelState) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, state: AudioLevelState) => callback(state);
    ipcRenderer.on('audio-level-state', listener);
    return () => ipcRenderer.removeListener('audio-level-state', listener);
  },
  onLibraryStateChanged: (callback: (state: LibraryState) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, state: LibraryState) => callback(state);
    ipcRenderer.on('library-state-changed', listener);
    return () => ipcRenderer.removeListener('library-state-changed', listener);
  },
  onDownloadProgressChanged: (callback: (progress: Map<string, string>) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, progress: [string, string][]) => callback(new Map(progress));
    ipcRenderer.on('download-progress-changed', listener);
    return () => ipcRenderer.removeListener('download-progress-changed', listener);
  },
  onNotification: (callback: (message: string) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, message: string) => callback(message);
    ipcRenderer.on('notification', listener);
    return () => ipcRenderer.removeListener('notification', listener);
  },

  // Sync events
  onLibrarySyncStarted: (callback: (data: { workspaceId: string | null }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { workspaceId: string | null }) => callback(data);
    ipcRenderer.on('library-sync-started', listener);
    return () => ipcRenderer.removeListener('library-sync-started', listener);
  },
  onLibrarySyncProgress: (callback: (data: { current: number; total: number }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { current: number; total: number }) => callback(data);
    ipcRenderer.on('library-sync-progress', listener);
    return () => ipcRenderer.removeListener('library-sync-progress', listener);
  },
  onLibrarySyncCompleted: (callback: (data: { workspaceId: string | null }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { workspaceId: string | null }) => callback(data);
    ipcRenderer.on('library-sync-completed', listener);
    return () => ipcRenderer.removeListener('library-sync-completed', listener);
  },
  onLibrarySyncFailed: (callback: (data: { error: string }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { error: string }) => callback(data);
    ipcRenderer.on('library-sync-failed', listener);
    return () => ipcRenderer.removeListener('library-sync-failed', listener);
  },

  onTrackLoadDeck: (callback: (data: { track: Track; deck: 1 | 2 }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { track: Track; deck: 1 | 2 }) => callback(data);
    ipcRenderer.on('track-load-deck', listener);
    return () => ipcRenderer.removeListener('track-load-deck', listener);
  },

  onWaveformChunk: (callback: (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => callback(data);
    ipcRenderer.on('waveform-chunk', listener);
    return () => ipcRenderer.removeListener('waveform-chunk', listener);
  },

  onWaveformComplete: (callback: (data: { trackId: string; totalFrames: number }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { trackId: string; totalFrames: number }) => callback(data);
    ipcRenderer.on('waveform-complete', listener);
    return () => ipcRenderer.removeListener('waveform-complete', listener);
  },

  onRecordingStatus: (callback: (status: RecordingStatus) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, status: RecordingStatus) => callback(status);
    ipcRenderer.on('recording-status', listener);
    return () => ipcRenderer.removeListener('recording-status', listener);
  },

});
// Types for window.electronAPI are declared in src/types/electron-api.d.ts
