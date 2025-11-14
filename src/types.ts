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
  pcmData?: Buffer;
  sampleRate?: number;
  channels?: number;
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
  deckA: Track | null;
  deckB: Track | null;
  deckAPosition: number;
  deckBPosition: number;
  deckAPlaying: boolean;
  deckBPlaying: boolean;
  isPlaying: boolean;
  isCrossfading: boolean;
  crossfadeProgress: number; // 0 = full A, 1 = full B
  crossfaderPosition: number; // Manual crossfader position (0-1)
  // For backward compatibility during migration
  currentTrack?: Track | null;
  nextTrack?: Track | null;
  position?: number;
  nextPosition?: number;
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
