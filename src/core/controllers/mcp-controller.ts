/**
 * MCP Controller - Bridges LibraryManager and Audio Worker for MCP server
 */

import type { LibraryManager } from '../library-manager';
import type { AudioEngineState, Workspace } from '../../types';
import type { AudioInfo } from '../../suno-api';
import type { WorkerInMsg, WorkerOutMsg } from '../../workers/audio-worker-types';

export class MCPController {
  private libraryManager: LibraryManager;
  private sendWorkerMessage: <T extends WorkerOutMsg>(msg: WorkerInMsg, timeout?: number) => Promise<T>;
  private lastAudioState: AudioEngineState | null = null;

  constructor(
    libraryManager: LibraryManager,
    sendWorkerMessage: <T extends WorkerOutMsg>(msg: WorkerInMsg, timeout?: number) => Promise<T>
  ) {
    this.libraryManager = libraryManager;
    this.sendWorkerMessage = sendWorkerMessage;
  }

  /**
   * Update cached audio state (called from main process)
   */
  updateAudioState(state: AudioEngineState): void {
    // Merge differential state updates into cached state
    if (!this.lastAudioState) {
      this.lastAudioState = {} as AudioEngineState;
    }

    // Update track info if provided (or explicitly set to null)
    if (state.deckA !== undefined) {
      this.lastAudioState.deckA = state.deckA;
    }
    if (state.deckB !== undefined) {
      this.lastAudioState.deckB = state.deckB;
    }

    // Update positions if provided
    if (state.deckAPosition !== undefined) {
      this.lastAudioState.deckAPosition = state.deckAPosition;
    }
    if (state.deckBPosition !== undefined) {
      this.lastAudioState.deckBPosition = state.deckBPosition;
    }

    // Update playing states (always present)
    this.lastAudioState.deckAPlaying = state.deckAPlaying;
    this.lastAudioState.deckBPlaying = state.deckBPlaying;
    this.lastAudioState.isPlaying = state.isPlaying;
    this.lastAudioState.isCrossfading = state.isCrossfading;
    this.lastAudioState.crossfadeProgress = state.crossfadeProgress;
    this.lastAudioState.crossfaderPosition = state.crossfaderPosition;

    // Update other fields if provided
    if (state.masterTempo !== undefined) {
      this.lastAudioState.masterTempo = state.masterTempo;
    }
    if (state.deckAEqCut !== undefined) {
      this.lastAudioState.deckAEqCut = state.deckAEqCut;
    }
    if (state.deckBEqCut !== undefined) {
      this.lastAudioState.deckBEqCut = state.deckBEqCut;
    }

    // Update peak levels (always present)
    this.lastAudioState.deckAPeak = state.deckAPeak;
    this.lastAudioState.deckBPeak = state.deckBPeak;
    this.lastAudioState.deckAPeakHold = state.deckAPeakHold;
    this.lastAudioState.deckBPeakHold = state.deckBPeakHold;
    this.lastAudioState.deckACueEnabled = state.deckACueEnabled;
    this.lastAudioState.deckBCueEnabled = state.deckBCueEnabled;
  }

  /**
   * Get list of workspaces
   */
  async listWorkspaces(): Promise<Workspace[]> {
    const state = this.libraryManager.getState();
    return state.workspaces;
  }

  /**
   * Get selected workspace
   */
  async getSelectedWorkspace(): Promise<Workspace | null> {
    const state = this.libraryManager.getState();
    return state.selectedWorkspace;
  }

  /**
   * Select workspace
   */
  async selectWorkspace(workspaceId: string | null): Promise<void> {
    const workspaces = await this.listWorkspaces();
    const workspace = workspaceId
      ? workspaces.find(w => w.id === workspaceId) || null
      : null;
    await this.libraryManager.setWorkspace(workspace);
  }

  /**
   * List all tracks in current workspace with metadata (excluding image data to save tokens)
   */
  async listTracks(): Promise<AudioInfo[]> {
    const state = this.libraryManager.getState();
    // Remove all image-related fields to avoid token waste
    return state.tracks.map(track => {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { image_url, image_large_url, cachedImageData, ...rest } = track as AudioInfo & { image_large_url?: string; cachedImageData?: string };
      return rest;
    });
  }

  /**
   * Load track to deck (downloads if needed)
   */
  async loadDeck(trackId: string, deck: 1 | 2): Promise<void> {
    // Check if deck is playing
    const state = await this.getAudioState();
    const isPlaying = deck === 1 ? state.deckAPlaying : state.deckBPlaying;
    if (isPlaying) {
      throw new Error(`Deck ${deck} is currently playing. Stop the deck before loading a new track.`);
    }

    const tracks = await this.listTracks();
    const audioInfo = tracks.find(t => t.id === trackId);
    if (!audioInfo) {
      throw new Error(`Track not found: ${trackId}`);
    }

    // Download track if not cached
    const track = await this.libraryManager.downloadTrack(audioInfo);

    // Load to deck via audio worker
    const res = await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'play',
      track,
      crossfade: false,
      targetDeck: deck,
    });

    if (res.type === 'playResult' && !res.ok) {
      throw new Error(res.error || 'Failed to load track');
    }
  }

  /**
   * Play deck
   */
  async playDeck(deck: 1 | 2): Promise<void> {
    const res = await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'startDeck',
      deck,
    });

    if (res.type === 'startDeckResult' && !res.ok) {
      throw new Error('Failed to start deck');
    }
  }

  /**
   * Stop deck
   */
  async stopDeck(deck: 1 | 2): Promise<void> {
    await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'stop',
      deck,
    });
  }

  /**
   * Set crossfader position (0 = full A, 1 = full B)
   */
  async setCrossfader(position: number): Promise<void> {
    await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'setCrossfader',
      position,
    });
  }

  /**
   * Get crossfader state
   */
  async getCrossfader(): Promise<{
    position: number;
    isCrossfading: boolean;
    autoProgress: number | null;
  }> {
    const res = await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'getState',
    });

    if (res.type === 'stateResult') {
      const state = res.state as AudioEngineState;
      return {
        position: state.crossfaderPosition,
        isCrossfading: state.isCrossfading,
        autoProgress: state.isCrossfading ? state.crossfadeProgress : null,
      };
    }

    return {
      position: this.lastAudioState?.crossfaderPosition || 0,
      isCrossfading: this.lastAudioState?.isCrossfading || false,
      autoProgress: this.lastAudioState?.isCrossfading
        ? this.lastAudioState.crossfadeProgress
        : null,
    };
  }

  /**
   * Trigger auto crossfade between currently loaded decks
   * @param targetPosition - Target crossfader position (0 = full A, 1 = full B). If null, automatically determined.
   * @param duration - Crossfade duration in seconds (default: 2)
   */
  async triggerCrossfade(
    targetPosition: number | null = null,
    duration = 2
  ): Promise<void> {
    // Trigger crossfade via audio worker
    const res = await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'startCrossfade',
      targetPosition: targetPosition ?? undefined,
      duration,
    });

    if (res.type === 'startCrossfadeResult' && !res.ok) {
      throw new Error(res.error || 'Failed to trigger crossfade');
    }
  }

  /**
   * Get deck information (loaded track, playing status, position, etc.)
   */
  async getDeckInfo(deck: 1 | 2): Promise<{
    track: { id: string; title: string; duration: number; bpm?: number } | null;
    isPlaying: boolean;
    position: number | null;
    remaining: number | null;
  }> {
    const state = await this.getAudioState();
    const track = deck === 1 ? state.deckA : state.deckB;
    const isPlaying = deck === 1 ? state.deckAPlaying : state.deckBPlaying;
    const position = deck === 1 ? state.deckAPosition : state.deckBPosition;

    if (!track) {
      return {
        track: null,
        isPlaying: false,
        position: null,
        remaining: null,
      };
    }

    const remaining = position !== undefined ? track.duration - position : null;

    return {
      track: {
        id: track.id,
        title: track.title,
        duration: track.duration,
        bpm: track.bpm,
      },
      isPlaying,
      position: position ?? null,
      remaining,
    };
  }

  /**
   * Set EQ cut (kill) state for a specific frequency band on a deck
   */
  async setEqCut(deck: 1 | 2, band: 'low' | 'mid' | 'high', enabled: boolean): Promise<void> {
    const res = await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'setEqCut',
      deck,
      band,
      enabled,
    });

    if (res.type === 'setEqCutResult' && !res.ok) {
      throw new Error(res.error || 'Failed to set EQ cut');
    }
  }

  /**
   * Get current EQ cut state for both decks
   */
  async getEqState(): Promise<{
    deckA: { low: boolean; mid: boolean; high: boolean };
    deckB: { low: boolean; mid: boolean; high: boolean };
  }> {
    const state = await this.getAudioState();
    return {
      deckA: state.deckAEqCut || { low: false, mid: false, high: false },
      deckB: state.deckBEqCut || { low: false, mid: false, high: false },
    };
  }

  /**
   * Get current master tempo in BPM
   */
  async getMasterTempo(): Promise<number> {
    const state = await this.getAudioState();
    return state.masterTempo || 130; // Default to 130 BPM if not set
  }

  /**
   * Set master tempo in BPM
   */
  async setMasterTempo(bpm: number): Promise<void> {
    if (bpm < 60 || bpm > 200) {
      throw new Error('Master tempo must be between 60 and 200 BPM');
    }

    await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'setMasterTempo',
      bpm,
    });
  }

  /**
   * Get playback time remaining for a deck
   */
  async getPlaybackTimeRemaining(deck: 1 | 2): Promise<number | null> {
    const state = await this.getAudioState();
    const track = deck === 1 ? state.deckA : state.deckB;
    const position = deck === 1 ? state.deckAPosition : state.deckBPosition;
    const isPlaying = deck === 1 ? state.deckAPlaying : state.deckBPlaying;

    if (!track || !isPlaying || position === undefined) {
      return null;
    }

    return track.duration - position;
  }

  /**
   * Wait until deck reaches specified position/time
   * This operation polls until the condition is met or timeout (10 seconds)
   * Returns true if condition met, false if timeout (caller should call again)
   */
  async waitUntilPosition(options: {
    deck: 1 | 2;
    remainingSeconds?: number;
    positionSeconds?: number;
    elapsedSeconds?: number;
  }): Promise<{ reached: boolean; currentPosition?: number; remaining?: number }> {
    const { deck, remainingSeconds, positionSeconds, elapsedSeconds } = options;

    // Validate that exactly one condition is specified
    const conditions = [remainingSeconds, positionSeconds, elapsedSeconds].filter(c => c !== undefined);
    if (conditions.length !== 1) {
      throw new Error('Must specify exactly one of: remainingSeconds, positionSeconds, or elapsedSeconds');
    }

    // Poll interval and timeout
    const pollInterval = 100;
    const timeout = 10000; // 10 seconds
    const startTime = Date.now();

    // eslint-disable-next-line no-constant-condition
    while (true) {
      const state = await this.getAudioState();
      const track = deck === 1 ? state.deckA : state.deckB;
      const position = deck === 1 ? state.deckAPosition : state.deckBPosition;
      const isPlaying = deck === 1 ? state.deckAPlaying : state.deckBPlaying;

      // If deck is not playing or no track loaded, exit
      if (!track || !isPlaying || position === undefined) {
        throw new Error(`Deck ${deck} is not playing`);
      }

      // Check condition
      let conditionMet = false;
      const remaining = track.duration - position;

      if (remainingSeconds !== undefined) {
        conditionMet = remaining <= remainingSeconds;
      } else if (positionSeconds !== undefined) {
        conditionMet = position >= positionSeconds;
      } else if (elapsedSeconds !== undefined) {
        conditionMet = position >= elapsedSeconds;
      }

      if (conditionMet) {
        return { reached: true, currentPosition: position, remaining };
      }

      // Check timeout
      if (Date.now() - startTime >= timeout) {
        return { reached: false, currentPosition: position, remaining };
      }

      // Wait before next poll
      await new Promise(resolve => setTimeout(resolve, pollInterval));
    }
  }

  /**
   * Get current audio state
   */
  async getAudioState(): Promise<AudioEngineState> {
    const res = await this.sendWorkerMessage<WorkerOutMsg>({
      type: 'getState',
    });

    if (res.type === 'stateResult') {
      // Merge differential update into cached state
      this.updateAudioState(res.state as AudioEngineState);
    }

    // Return the fully merged state
    return this.lastAudioState || ({} as AudioEngineState);
  }
}
