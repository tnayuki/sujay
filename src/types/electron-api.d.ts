/**
 * Type definitions for Electron API exposed via contextBridge
 * This file augments the global Window interface with electronAPI
 */

import type { AudioEngineState, LibraryState, GenerationState, Track, Workspace } from '../types';
import type { AudioInfo } from '../suno-api';

export interface ElectronAPI {
  // Audio Engine
  audioPlay: (track: Track, crossfade: boolean) => Promise<void>;
  audioStop: () => Promise<void>;
  audioGetState: () => Promise<AudioEngineState>;
  onAudioStateChanged: (callback: (state: AudioEngineState) => void) => () => void;

  // Library Manager
  libraryGetState: () => Promise<LibraryState>;
  libraryGetDownloadProgress: () => Promise<[string, string][]>;
  libraryDownloadTrack: (audioInfo: AudioInfo) => Promise<Track>;
  libraryNextPage: () => Promise<void>;
  libraryPreviousPage: () => Promise<void>;
  librarySetWorkspace: (workspace: Workspace | null) => Promise<void>;
  libraryToggleLikedFilter: () => Promise<void>;
  onLibraryStateChanged: (callback: (state: LibraryState) => void) => () => void;
  onDownloadProgressChanged: (callback: (progress: Map<string, string>) => void) => () => void;

  // Generation Manager
  generationGetState: () => Promise<GenerationState>;
  generationGenerate: (prompt: string) => Promise<void>;
  onGenerationStateChanged: (callback: (state: GenerationState) => void) => () => void;

  // Notifications
  onNotification: (callback: (message: string) => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
