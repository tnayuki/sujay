/**
 * Type definitions for Electron API exposed via contextBridge
 * This file augments the global Window interface with electronAPI
 */

import type {
  AudioEngineState,
  Track,
  OSCConfig,
  AudioConfig,
  AudioDevice,
  RecordingConfig,
  RecordingStatus,
  EqBand,
  TrackStructure,
} from '../types';

type NativeUIFrame = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export interface ElectronAPI {
  audioLoadTrack: (track: Track, deck: 1 | 2) => Promise<void>;
  audioPlay: (track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null) => Promise<void>;
  audioStop: (deck: 1 | 2) => Promise<void>;
  audioGetState: () => Promise<AudioEngineState>;
  audioSeek: (deck: 1 | 2, position: number) => Promise<void>;
  audioSetCrossfader: (position: number) => Promise<void>;
  audioSetMasterTempo: (bpm: number) => Promise<void>;
  audioSetDeckCue: (deck: 1 | 2, enabled: boolean) => Promise<void>;
  audioSetEqCut: (deck: 1 | 2, band: EqBand, enabled: boolean) => Promise<void>;
  audioSetDeckGain: (deck: 1 | 2, gain: number) => Promise<void>;
  audioStartDeck: (deck: 1 | 2) => Promise<void>;
  audioSetMicEnabled: (enabled: boolean) => Promise<void>;
  audioSetBeatLoop: (deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) => Promise<void>;
  audioClearLoop: (deck: 1 | 2) => Promise<void>;
  audioGetDevices: () => Promise<AudioDevice[]>;
  audioGetConfig: () => Promise<AudioConfig>;
  audioUpdateConfig: (config: AudioConfig) => Promise<void>;

  oscGetConfig: () => Promise<OSCConfig>;
  oscUpdateConfig: (config: OSCConfig) => Promise<void>;

  recordingGetConfig: () => Promise<RecordingConfig>;
  recordingUpdateConfig: (config: RecordingConfig) => Promise<RecordingConfig>;
  recordingGetStatus: () => Promise<RecordingStatus>;
  recordingStart: (format: 'wav' | 'ogg') => Promise<RecordingStatus>;
  recordingStop: () => Promise<RecordingStatus>;

  nativeUiAttach: (frame: NativeUIFrame) => Promise<boolean>;
  nativeUiSetFrame: (frame: NativeUIFrame) => Promise<boolean>;
  nativeUiSetWaveform: (deck: 1 | 2, samples: number[]) => Promise<boolean>;
  nativeUiSetArtwork: (deck: 1 | 2, width: number, height: number, rgba: Uint8Array) => Promise<void>;
  nativeUiClearArtwork: (deck: 1 | 2) => Promise<void>;
  nativeUiDetach: () => Promise<boolean>;

  getSystemInfo: () => Promise<{ time: string; cpuUsage: number; memoryUsage: number }>;
  onWaveformLoaded: (callback: (data: { deck: 1 | 2; trackId: string; waveformData: Float32Array | number[] }) => void) => () => void;
  onWaveformChunk: (callback: (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => void) => () => void;
  onWaveformComplete: (callback: (data: { trackId: string; totalFrames: number }) => void) => () => void;
  onTrackStructure: (callback: (data: { trackId: string; deck: 1 | 2; structure: TrackStructure }) => void) => () => void;
  onNotification: (callback: (message: string) => void) => () => void;
  onRecordingStatus: (callback: (status: RecordingStatus) => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
