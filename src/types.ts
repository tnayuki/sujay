/**
 * Shared type definitions for Sujay
 */

import type { AudioInfo } from './suno-api';

export interface Track {
  id: string;
  title: string;
  mp3Path: string;
  duration: number;
  is_liked?: boolean;
  gpt_description_prompt?: string;
  tags?: string;
  pcmData?: Float32Array;
  sampleRate?: number;
  channels?: number;
  bpm?: number; // Detected or user-provided BPM
  float32Mono?: Float32Array; // PCM (mono, -1.0 to 1.0) for BPM detection
  waveform?: number[]; // Deprecated: use waveformData instead
  waveformData?: number[]; // Full PCM data as normalized floats (-1 to 1)
}

export interface Workspace {
  id: string;
  name: string;
}

/**
 * Audio Engine State
 */
export interface AudioEngineState {
  deckA?: Track | null; // Only included when track changes
  deckB?: Track | null; // Only included when track changes
  deckAPosition?: number; // Only included when position changes (seek)
  deckBPosition?: number; // Only included when position changes (seek)
  isSeek?: boolean; // True when position change is from seek operation
  deckAPlaying: boolean;
  deckBPlaying: boolean;
  isPlaying: boolean;
  isCrossfading: boolean;
  crossfadeProgress: number; // 0 = full A, 1 = full B
  crossfaderPosition: number; // Manual crossfader position (0-1)
  masterTempo?: number; // Master tempo in BPM (included only when changed)
  deckALevel: number; // RMS level 0-1
  deckBLevel: number; // RMS level 0-1
  // For backward compatibility during migration
  currentTrack?: Track | null;
  nextTrack?: Track | null;
  position?: number;
  nextPosition?: number;
}

/**
 * Audio Level State (high-frequency updates for level meters)
 */
export interface AudioLevelState {
  deckALevel: number;
  deckBLevel: number;
}

/**
 * Audio Engine Events
 */
export type AudioEngineEventMap = {
  'state-changed': AudioEngineState;
  'track-ended': void;
  'error': Error;
  'waveform-chunk': {
    trackId: string;
    chunkIndex: number;
    totalChunks: number;
    chunk: number[];
  };
  'waveform-complete': {
    trackId: string;
    totalFrames: number;
  };
};

/**
 * Library Manager State
 */
export interface LibraryState {
  tracks: AudioInfo[];
  workspaces: Workspace[];
  selectedWorkspace: Workspace | null;
  likedFilter: boolean;
  syncing: boolean;
}

/**
 * OSC Configuration
 */
export interface OSCConfig {
  enabled: boolean;
  host: string;
  port: number;
}

/**
 * Audio Device Configuration
 */
export interface AudioConfig {
  deviceId?: number;
  // Use null to indicate "not routed" for that side
  mainChannels: [number | null, number | null]; // [left, right] channel indices for main output
  cueChannels: [number | null, number | null];  // [left, right] channel indices for cue output
}

/**
 * Audio Device Information
 */
export interface AudioDevice {
  id: number;
  name: string;
  maxOutputChannels: number;
}
