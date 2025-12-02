/**
 * Shared type definitions for Sujay
 */

import type { AudioInfo } from './suno-api';

/**
 * Track structure section (intro/main/outro)
 */
export interface TrackSection {
  start: number;  // Start position in seconds
  end: number;    // End position in seconds
  beats: number;  // Number of beats in this section
}

/**
 * Track structure analysis result
 */
export interface TrackStructure {
  bpm: number;
  beats: number[];    // Beat positions in seconds
  intro: TrackSection;
  main: TrackSection;
  outro: TrackSection;
  hotCues: number[];  // Important positions in seconds (first beat, intro end, outro start, etc.)
}

export interface Track {
  id: string;
  title: string;
  mp3Path: string;
  duration: number;
  is_liked?: boolean;
  gpt_description_prompt?: string;
  tags?: string;
  image_url?: string; // Image URL from Suno API
  cachedImageData?: string; // Base64 data URL for cached image
  pcmData?: Float32Array;
  sampleRate?: number;
  channels?: number;
  bpm?: number; // BPM detected by the audio engine (not provided by metadata)
  float32Mono?: Float32Array; // PCM (mono, -1.0 to 1.0) for BPM detection
  waveform?: number[]; // Deprecated: use waveformData instead
  waveformData?: number[] | Float32Array; // Full PCM data as normalized floats (-1 to 1)
  structure?: TrackStructure; // Track structure analysis (intro/outro/main sections)
}

export interface Workspace {
  id: string;
  name: string;
}

/**
 * EQ Band Identifiers
 */
export type EqBand = 'low' | 'mid' | 'high';

/**
 * EQ Cut State (kill switches)
 */
export interface EqCutState {
  low: boolean;
  mid: boolean;
  high: boolean;
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
  deckAPeak: number; // Peak level 0-1
  deckBPeak: number; // Peak level 0-1
  deckAPeakHold: number; // Peak hold level 0-1
  deckBPeakHold: number; // Peak hold level 0-1
  deckACueEnabled: boolean;
  deckBCueEnabled: boolean;
  micAvailable?: boolean;
  micEnabled?: boolean;
  micWarning?: string | null;
  talkoverActive?: boolean;
  talkoverButtonPressed?: boolean; // Manual talkover trigger
  micLevel?: number;
  deckAEqCut?: EqCutState; // Deck A EQ kill state
  deckBEqCut?: EqCutState; // Deck B EQ kill state
  deckAGain?: number; // Deck A gain (0-1)
  deckBGain?: number; // Deck B gain (0-1)
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
  deckAPeak: number;
  deckBPeak: number;
  deckAPeakHold: number;
  deckBPeakHold: number;
  micLevel: number;
  talkoverActive: boolean;
  talkoverButtonPressed?: boolean;
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
  deviceId?: string;  // Device name (stable across restarts)
  // Use null to indicate "not routed" for that side
  mainChannels: [number | null, number | null]; // [left, right] channel indices for main output
  cueChannels: [number | null, number | null];  // [left, right] channel indices for cue output
}

/**
 * Audio Device Information
 */
export interface AudioDevice {
  name: string;  // Device name (stable across restarts, used as ID)
  maxOutputChannels: number;
}

/**
 * Recording Configuration
 */
export interface RecordingConfig {
  /** Absolute path to the directory where WAV files will be stored */
  directory: string;
  /** Whether the app should create the directory automatically when missing */
  autoCreateDirectory: boolean;
  /** Naming strategy for generated files (timestamp preferred, counter fallback) */
  namingStrategy: 'timestamp' | 'sequential';
}

export type RecordingState = 'idle' | 'preparing' | 'recording' | 'stopping' | 'error';

export interface RecordingFileInfo {
  path: string;
  createdAt: number; // epoch milliseconds
  bytesWritten: number;
}

export interface RecordingStatus {
  state: RecordingState;
  activeFile?: RecordingFileInfo;
  lastError?: string;
}

export interface SunoConfig {
  cookie: string;
}
