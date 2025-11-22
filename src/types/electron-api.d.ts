/**
 * Type definitions for Electron API exposed via contextBridge
 * This file augments the global Window interface with electronAPI
 */

import type { AudioEngineState, AudioLevelState, LibraryState, Track, Workspace, OSCConfig, AudioConfig, AudioDevice } from '../types';
import type { AudioInfo } from '../suno-api';

export interface ElectronAPI {
  audioPlay: (track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null) => Promise<void>;
  audioStop: (deck: 1 | 2) => Promise<void>;
  audioGetState: () => Promise<AudioEngineState>;
  audioSeek: (deck: 1 | 2, position: number) => Promise<void>;
  audioSetCrossfader: (position: number) => Promise<void>;
  audioSetMasterTempo: (bpm: number) => Promise<void>;
  audioSetDeckCue: (deck: 1 | 2, enabled: boolean) => Promise<void>;
  audioStartDeck: (deck: 1 | 2) => Promise<void>;
  audioGetDevices: () => Promise<AudioDevice[]>;
  audioGetConfig: () => Promise<AudioConfig>;
  audioUpdateConfig: (config: AudioConfig) => Promise<void>;
  onAudioStateChanged: (callback: (state: AudioEngineState) => void) => () => void;
  onAudioLevelState: (callback: (state: AudioLevelState) => void) => () => void;

  oscGetConfig: () => Promise<OSCConfig>;
  oscUpdateConfig: (config: OSCConfig) => Promise<void>;

  libraryGetState: () => Promise<LibraryState>;
  libraryGetDownloadProgress: () => Promise<[string, string][]>;
  libraryDownloadTrack: (audioInfo: AudioInfo) => Promise<Track>;
  librarySetWorkspace: (workspace: Workspace | null) => Promise<void>;
  librarySetLikedFilter: (enabled: boolean) => Promise<void>;
  libraryToggleLikedFilter: () => Promise<void>;
  showTrackContextMenu: (track: AudioInfo) => void;
  getSystemInfo: () => Promise<{ time: string; cpuUsage: number }>;
  onLibraryStateChanged: (callback: (state: LibraryState) => void) => () => void;
  onDownloadProgressChanged: (callback: (progress: Map<string, string>) => void) => () => void;
  onLibrarySyncStarted: (callback: (data) => void) => () => void;
  onLibrarySyncProgress: (callback: (data) => void) => () => void;
  onLibrarySyncCompleted: (callback: (data) => void) => () => void;
  onLibrarySyncFailed: (callback: (data) => void) => () => void;

  onTrackLoadDeck: (callback: (data: { track: AudioInfo; deck: 1 | 2 }) => void) => () => void;
  onWaveformChunk: (callback: (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => void) => () => void;
  onWaveformComplete: (callback: (data: { trackId: string; totalFrames: number }) => void) => () => void;
  onNotification: (callback: (message: string) => void) => () => void;
  onOpenPreferences: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
