/**
 * Type definitions for audio worker messages
 */

import type { Track, AudioEngineState, AudioLevelState, AudioConfig, OSCConfig, AudioDevice } from '../types';

// Incoming messages from main to worker
export type WorkerInMsg =
  | { type: 'ping'; id?: number }
  | { type: 'probeDevices'; id?: number }
  | { type: 'getDevices'; id?: number }
  | { type: 'init'; id?: number; audioConfig: AudioConfig; oscConfig: OSCConfig }
  | { type: 'play'; id?: number; track: Track; crossfade?: boolean; targetDeck?: 1 | 2 | null }
  | { type: 'stop'; id?: number; deck: 1 | 2 }
  | { type: 'seek'; id?: number; deck: 1 | 2; position: number }
  | { type: 'setCrossfader'; id?: number; position: number }
  | { type: 'setMasterTempo'; id?: number; bpm: number }
  | { type: 'startDeck'; id?: number; deck: 1 | 2 }
  | { type: 'getState'; id?: number }
  | { type: 'updateOSCConfig'; id?: number; config: OSCConfig }
  | { type: 'applyAudioConfig'; id?: number; config: AudioConfig }
  | { type: 'cleanup'; id?: number };

// Outgoing messages from worker to main
export type WorkerOutMsg =
  | { type: 'pong'; id?: number }
  | { type: 'probeResult'; id?: number; ok: boolean; count?: number; error?: string }
  | { type: 'devices'; id?: number; devices: AudioDevice[] }
  | { type: 'initResult'; id?: number; ok: boolean; error?: string }
  | { type: 'playResult'; id?: number; ok: boolean; error?: string }
  | { type: 'stopResult'; id?: number; ok: boolean }
  | { type: 'seekResult'; id?: number; ok: boolean }
  | { type: 'setCrossfaderResult'; id?: number; ok: boolean }
  | { type: 'setMasterTempoResult'; id?: number; ok: boolean }
  | { type: 'startDeckResult'; id?: number; ok: boolean }
  | { type: 'stateResult'; id?: number; state: AudioEngineState }
  | { type: 'updateOSCConfigResult'; id?: number; ok: boolean }
  | { type: 'applyAudioConfigResult'; id?: number; ok: boolean; error?: string }
  | { type: 'cleanupResult'; id?: number; ok: boolean }
  // Events from AudioEngine
  | { type: 'stateChanged'; state: AudioEngineState }
  | { type: 'levelState'; state: AudioLevelState }
  | { type: 'trackEnded' }
  | { type: 'error'; error: string }
  | { type: 'waveformChunk'; trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }
  | { type: 'waveformComplete'; trackId: string; totalFrames: number };
