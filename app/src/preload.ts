/**
 * Electron Preload Script
 * Exposes secure IPC bridge to renderer process
 */

import { contextBridge, ipcRenderer } from 'electron';
import type {
  Track,
  OSCConfig,
  AudioConfig,
  RecordingConfig,
  RecordingStatus,
  EqBand,
  TrackStructure,
} from './types';

type NativeUIFrame = {
  x: number;
  y: number;
  width: number;
  height: number;
};

// Expose protected methods that allow the renderer process to use
// the ipcRenderer without exposing the entire object
contextBridge.exposeInMainWorld('electronAPI', {
  // Audio Engine
  audioLoadTrack: (track: Track, deck: 1 | 2) => ipcRenderer.invoke('audio:load-track', track, deck),
  audioPlay: (track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null) => ipcRenderer.invoke('audio:play', track, crossfade, targetDeck ?? null),
  audioStop: (deck: 1 | 2) => ipcRenderer.invoke('audio:stop', deck),
  audioGetState: () => ipcRenderer.invoke('audio:get-state'),
  audioSeek: (deck: 1 | 2, position: number) => ipcRenderer.invoke('audio:seek', deck, position),
  audioSetCrossfader: (position: number) => ipcRenderer.invoke('audio:set-crossfader', position),
  audioSetMasterTempo: (bpm: number) => ipcRenderer.invoke('audio:set-master-tempo', bpm),
  audioSetDeckCue: (deck: 1 | 2, enabled: boolean) => ipcRenderer.invoke('audio:set-deck-cue', deck, enabled),
  audioSetEqCut: (deck: 1 | 2, band: EqBand, enabled: boolean) => ipcRenderer.invoke('audio:set-eq-cut', deck, band, enabled),
  audioSetDeckGain: (deck: 1 | 2, gain: number) => ipcRenderer.invoke('audio:set-deck-gain', deck, gain),
  audioStartDeck: (deck: 1 | 2) => ipcRenderer.invoke('audio:start-deck', deck),
  audioSetMicEnabled: (enabled: boolean) => ipcRenderer.invoke('audio:set-mic-enabled', enabled),
  audioSetBeatLoop: (deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) => ipcRenderer.invoke('audio:set-beat-loop', deck, beats, masterTempo, currentPosition, beatGrid),
  audioClearLoop: (deck: 1 | 2) => ipcRenderer.invoke('audio:clear-loop', deck),
  
  // Audio Config
  audioGetDevices: () => ipcRenderer.invoke('audio:get-devices'),
  audioGetConfig: () => ipcRenderer.invoke('audio:get-config'),
  audioUpdateConfig: (config: AudioConfig) => ipcRenderer.invoke('audio:update-config', config),

  // System info
  getSystemInfo: () => ipcRenderer.invoke('system:get-info'),

  // OSC Config
  oscGetConfig: () => ipcRenderer.invoke('osc:get-config'),
  oscUpdateConfig: (config: OSCConfig) => ipcRenderer.invoke('osc:update-config', config),

  // Recording
  recordingGetConfig: () => ipcRenderer.invoke('recording:get-config'),
  recordingUpdateConfig: (config: RecordingConfig) => ipcRenderer.invoke('recording:update-config', config),
  recordingGetStatus: () => ipcRenderer.invoke('recording:get-status'),
  recordingStart: (format: 'wav' | 'ogg') => ipcRenderer.invoke('recording:start', format),
  recordingStop: () => ipcRenderer.invoke('recording:stop'),

  // Native UI
  nativeUiAttach: (frame: NativeUIFrame) => ipcRenderer.invoke('native-ui:attach', frame),
  nativeUiSetFrame: (frame: NativeUIFrame) => ipcRenderer.invoke('native-ui:set-frame', frame),
  nativeUiSetWaveform: (deck: 1 | 2, samples: number[]) => ipcRenderer.invoke('native-ui:set-waveform', deck, samples),
  nativeUiSetArtwork: (deck: 1 | 2, width: number, height: number, rgba: Uint8Array) => ipcRenderer.invoke('native-ui:set-artwork', deck, width, height, Buffer.from(rgba)),
  nativeUiClearArtwork: (deck: 1 | 2) => ipcRenderer.invoke('native-ui:clear-artwork', deck),
  nativeUiDetach: () => ipcRenderer.invoke('native-ui:detach'),

  // Event listeners - return cleanup functions
  onNotification: (callback: (message: string) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, message: string) => callback(message);
    ipcRenderer.on('notification', listener);
    return () => ipcRenderer.removeListener('notification', listener);
  },

  onWaveformLoaded: (callback: (data: { deck: 1 | 2; trackId: string; waveformData: Float32Array | number[] }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { deck: 1 | 2; trackId: string; waveformData: Float32Array | number[] }) => callback(data);
    ipcRenderer.on('waveform-loaded', listener);
    return () => ipcRenderer.removeListener('waveform-loaded', listener);
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

  onTrackStructure: (callback: (data: { trackId: string; deck: 1 | 2; structure: TrackStructure }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { trackId: string; deck: 1 | 2; structure: TrackStructure }) => callback(data);
    ipcRenderer.on('track-structure', listener);
    return () => ipcRenderer.removeListener('track-structure', listener);
  },

  onRecordingStatus: (callback: (status: RecordingStatus) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, status: RecordingStatus) => callback(status);
    ipcRenderer.on('recording-status', listener);
    return () => ipcRenderer.removeListener('recording-status', listener);
  },

});
// Types for window.electronAPI are declared in src/types/electron-api.d.ts
