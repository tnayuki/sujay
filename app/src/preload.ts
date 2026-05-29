/**
 * Electron Preload Script
 * Exposes secure IPC bridge to renderer process
 */

import { contextBridge, ipcRenderer } from 'electron';
import { IPC_CHANNELS, IPC_EVENTS } from './main/ipc-contract';
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
  audioLoadTrack: (track: Track, deck: 1 | 2) => ipcRenderer.invoke(IPC_CHANNELS.audio.loadTrack, track, deck),
  audioPlay: (track: Track, crossfade: boolean, targetDeck?: 1 | 2 | null) => ipcRenderer.invoke(IPC_CHANNELS.audio.play, track, crossfade, targetDeck ?? null),
  audioStop: (deck: 1 | 2) => ipcRenderer.invoke(IPC_CHANNELS.audio.stop, deck),
  audioGetState: () => ipcRenderer.invoke(IPC_CHANNELS.audio.getState),
  audioSeek: (deck: 1 | 2, position: number) => ipcRenderer.invoke(IPC_CHANNELS.audio.seek, deck, position),
  audioSetCrossfader: (position: number) => ipcRenderer.invoke(IPC_CHANNELS.audio.setCrossfader, position),
  audioSetMasterTempo: (bpm: number) => ipcRenderer.invoke(IPC_CHANNELS.audio.setMasterTempo, bpm),
  audioSetDeckCue: (deck: 1 | 2, enabled: boolean) => ipcRenderer.invoke(IPC_CHANNELS.audio.setDeckCue, deck, enabled),
  audioSetEqCut: (deck: 1 | 2, band: EqBand, enabled: boolean) => ipcRenderer.invoke(IPC_CHANNELS.audio.setEqCut, deck, band, enabled),
  audioSetDeckGain: (deck: 1 | 2, gain: number) => ipcRenderer.invoke(IPC_CHANNELS.audio.setDeckGain, deck, gain),
  audioStartDeck: (deck: 1 | 2) => ipcRenderer.invoke(IPC_CHANNELS.audio.startDeck, deck),
  audioSetMicEnabled: (enabled: boolean) => ipcRenderer.invoke(IPC_CHANNELS.audio.setMicEnabled, enabled),
  audioSetBeatLoop: (deck: 1 | 2, beats: number, masterTempo: number, currentPosition: number, beatGrid?: number[]) => ipcRenderer.invoke(IPC_CHANNELS.audio.setBeatLoop, deck, beats, masterTempo, currentPosition, beatGrid),
  audioClearLoop: (deck: 1 | 2) => ipcRenderer.invoke(IPC_CHANNELS.audio.clearLoop, deck),
  
  // Audio Config
  audioGetDevices: () => ipcRenderer.invoke(IPC_CHANNELS.audio.getDevices),
  audioGetConfig: () => ipcRenderer.invoke(IPC_CHANNELS.audio.getConfig),
  audioUpdateConfig: (config: AudioConfig) => ipcRenderer.invoke(IPC_CHANNELS.audio.updateConfig, config),

  // System info
  getSystemInfo: () => ipcRenderer.invoke(IPC_CHANNELS.system.getInfo),

  // OSC Config
  oscGetConfig: () => ipcRenderer.invoke(IPC_CHANNELS.osc.getConfig),
  oscUpdateConfig: (config: OSCConfig) => ipcRenderer.invoke(IPC_CHANNELS.osc.updateConfig, config),

  // Recording
  recordingGetConfig: () => ipcRenderer.invoke(IPC_CHANNELS.recording.getConfig),
  recordingUpdateConfig: (config: RecordingConfig) => ipcRenderer.invoke(IPC_CHANNELS.recording.updateConfig, config),
  recordingGetStatus: () => ipcRenderer.invoke(IPC_CHANNELS.recording.getStatus),
  recordingStart: (format: 'wav' | 'ogg') => ipcRenderer.invoke(IPC_CHANNELS.recording.start, format),
  recordingStop: () => ipcRenderer.invoke(IPC_CHANNELS.recording.stop),

  // Native UI
  nativeUiAttach: (frame: NativeUIFrame) => ipcRenderer.invoke(IPC_CHANNELS.nativeUi.attach, frame),
  nativeUiSetFrame: (frame: NativeUIFrame) => ipcRenderer.invoke(IPC_CHANNELS.nativeUi.setFrame, frame),
  nativeUiSetWaveform: (deck: 1 | 2, samples: number[]) => ipcRenderer.invoke(IPC_CHANNELS.nativeUi.setWaveform, deck, samples),
  nativeUiSetArtwork: (deck: 1 | 2, width: number, height: number, rgba: Uint8Array) => ipcRenderer.invoke(IPC_CHANNELS.nativeUi.setArtwork, deck, width, height, Buffer.from(rgba)),
  nativeUiClearArtwork: (deck: 1 | 2) => ipcRenderer.invoke(IPC_CHANNELS.nativeUi.clearArtwork, deck),
  nativeUiDetach: () => ipcRenderer.invoke(IPC_CHANNELS.nativeUi.detach),

  // Event listeners - return cleanup functions
  onNotification: (callback: (message: string) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, message: string) => callback(message);
    ipcRenderer.on(IPC_EVENTS.notification, listener);
    return () => ipcRenderer.removeListener(IPC_EVENTS.notification, listener);
  },

  onWaveformLoaded: (callback: (data: { deck: 1 | 2; trackId: string; waveformData: Float32Array | number[] }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { deck: 1 | 2; trackId: string; waveformData: Float32Array | number[] }) => callback(data);
    ipcRenderer.on('waveform-loaded', listener);
    return () => ipcRenderer.removeListener('waveform-loaded', listener);
  },

  onWaveformChunk: (callback: (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => callback(data);
    ipcRenderer.on(IPC_EVENTS.waveformChunk, listener);
    return () => ipcRenderer.removeListener(IPC_EVENTS.waveformChunk, listener);
  },

  onWaveformComplete: (callback: (data: { trackId: string; totalFrames: number }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { trackId: string; totalFrames: number }) => callback(data);
    ipcRenderer.on(IPC_EVENTS.waveformComplete, listener);
    return () => ipcRenderer.removeListener(IPC_EVENTS.waveformComplete, listener);
  },

  onTrackStructure: (callback: (data: { trackId: string; deck: 1 | 2; structure: TrackStructure }) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: { trackId: string; deck: 1 | 2; structure: TrackStructure }) => callback(data);
    ipcRenderer.on(IPC_EVENTS.trackStructure, listener);
    return () => ipcRenderer.removeListener(IPC_EVENTS.trackStructure, listener);
  },

  onRecordingStatus: (callback: (status: RecordingStatus) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, status: RecordingStatus) => callback(status);
    ipcRenderer.on(IPC_EVENTS.recordingStatus, listener);
    return () => ipcRenderer.removeListener(IPC_EVENTS.recordingStatus, listener);
  },

});
// Types for window.electronAPI are declared in src/types/electron-api.d.ts
