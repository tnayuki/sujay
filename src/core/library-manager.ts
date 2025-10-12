/**
 * Library Manager - Handles Suno API interaction, caching, and library state
 */

import { EventEmitter } from 'events';
import { sunoApi } from '../suno-api.js';
import type { AudioInfo } from '../suno-api.js';
import path from 'path';
import { promises as fs, existsSync } from 'fs';
import axios from 'axios';
// Dynamic import for is-online to handle ESM properly
async function isOnline(): Promise<boolean> {
  const { default: check } = await import('is-online');
  return await check();
}
import type { Track, Workspace, LibraryState } from '../types.js';
import { MetadataCache } from './metadata-cache.js';

export class LibraryManager extends EventEmitter {
  private client: any = null;
  private metadataCache: MetadataCache;

  // Library state
  private allTracks: AudioInfo[] = [];
  private state: LibraryState = {
    tracks: [],
    workspaces: [],
    selectedWorkspace: null,
    likedFilter: false,
    syncing: false,
  };

  private cacheDir: string;
  private downloadProgress: Map<string, string> = new Map();

  constructor(cacheDir: string, private cookie: string) {
    super();
    this.cacheDir = cacheDir;
    this.metadataCache = new MetadataCache(cacheDir);
  }

  /**
   * Initialize library manager
   */
  async initialize(): Promise<void> {
    // Ensure cache directory exists
    await fs.mkdir(this.cacheDir, { recursive: true });

    // Initialize Suno API client
    this.client = await sunoApi(this.cookie);

    // Load from cache first (instant display)
    await this.loadFromCache();

    // Background sync from API if online
    const online = await isOnline();
    if (online) {
      this.syncFromAPI().catch(error => {
        console.error('Background sync failed:', error);
      });
    }
  }

  /**
   * Load data from cache
   */
  private async loadFromCache(): Promise<void> {
    // Load workspaces
    const workspaces = await this.metadataCache.getWorkspaces();
    if (workspaces && workspaces.length > 0) {
      this.state.workspaces = workspaces;

      // Only set selectedWorkspace if not already set
      if (!this.state.selectedWorkspace) {
        this.state.selectedWorkspace = workspaces[0];
      }
    }

    // Load tracks for current workspace
    const workspaceId = this.state.selectedWorkspace?.id || null;
    const tracks = await this.metadataCache.getWorkspaceTracks(workspaceId);
    if (tracks) {
      this.allTracks = tracks;
      this.applyFilters();
    }
  }

  /**
   * Sync data from API (background)
   */
  private async syncFromAPI(): Promise<void> {
    const workspaceId = this.state.selectedWorkspace?.id || null;

    try {
      this.state.syncing = true;
      this.emit('sync-started', { workspaceId });
      this.emitState();

      // Load workspaces
      const workspaces = await this.client.getWorkspaces(1);
      this.state.workspaces = workspaces.map((w: any) => ({
        id: w.id,
        name: w.name,
      }));

      // Set default workspace if not set
      if (!this.state.selectedWorkspace && this.state.workspaces.length > 0) {
        this.state.selectedWorkspace = this.state.workspaces[0];
      }

      await this.metadataCache.saveWorkspaces(this.state.workspaces);

      // Load all tracks for current workspace (with pagination)
      const allTracks: AudioInfo[] = [];
      let cursor: string | null = null;
      let pageCount = 0;

      do {
        const result = await this.client.getFeedV3(
          workspaceId,
          cursor,
          100, // Fetch 100 at a time
          false // Don't filter by liked (we'll do that client-side)
        );

        allTracks.push(...result.clips);
        cursor = result.cursor;
        pageCount++;

        // Emit progress
        this.emit('sync-progress', {
          workspaceId,
          current: allTracks.length,
          message: `Fetching page ${pageCount}...`,
        });
      } while (cursor);

      // Save to cache
      console.log('[LibraryManager] Saving', allTracks.length, 'tracks to cache...');
      await this.metadataCache.saveWorkspaceTracks(workspaceId, allTracks);
      console.log('[LibraryManager] Tracks saved to cache');
      this.allTracks = allTracks;
      this.applyFilters();

      this.state.syncing = false;
      console.log('[LibraryManager] Sync completed, total:', allTracks.length);
      this.emit('sync-completed', { workspaceId, total: allTracks.length });
      this.emitState();

    } catch (error: any) {
      this.state.syncing = false;
      this.emit('sync-failed', { error: error.message });
      this.emitState();
      throw error;
    }
  }

  /**
   * Apply filters and update displayed tracks
   */
  private applyFilters(): void {
    let filtered = [...this.allTracks];

    // Apply liked filter
    if (this.state.likedFilter) {
      filtered = filtered.filter(t => t.is_liked);
    }

    // Update state
    this.state.tracks = filtered;
    this.emitState();
  }

  /**
   * Set workspace filter
   */
  async setWorkspace(workspace: Workspace | null): Promise<void> {
    this.state.selectedWorkspace = workspace;

    // Load from cache
    await this.loadFromCache();

    // Background sync if online
    const online = await isOnline();
    if (online) {
      this.syncFromAPI().catch(error => {
        console.error('Background sync failed:', error);
      });
    }
  }

  /**
   * Set liked filter
   */
  async setLikedFilter(enabled: boolean): Promise<void> {
    this.state.likedFilter = enabled;
    this.applyFilters();
  }

  /**
   * Toggle liked filter
   */
  async toggleLikedFilter(): Promise<void> {
    this.state.likedFilter = !this.state.likedFilter;
    this.applyFilters();
  }

  /**
   * Reload current workspace
   */
  async reload(): Promise<void> {
    const online = await isOnline();
    if (online) {
      await this.syncFromAPI();
    } else {
      await this.loadFromCache();
    }
  }

  /**
   * Download and cache track image
   */
  private async downloadImage(audioInfo: AudioInfo): Promise<string | null> {
    if (!audioInfo.image_url) return null;

    const imagePath = path.join(this.cacheDir, `${audioInfo.id}.jpg`);

    // Check if already cached
    if (existsSync(imagePath)) {
      return imagePath;
    }

    try {
      const response = await axios.get(audioInfo.image_url, {
        responseType: 'arraybuffer',
      });
      await fs.writeFile(imagePath, Buffer.from(response.data));
      return imagePath;
    } catch (error) {
      console.error(`Failed to download image for ${audioInfo.id}:`, error);
      return null;
    }
  }

  /**
   * Download and cache a track
   */
  async downloadTrack(audioInfo: AudioInfo): Promise<Track> {
    const mp3Path = path.join(this.cacheDir, `${audioInfo.id}.mp3`);

    // Check if already cached
    if (existsSync(mp3Path)) {
      return this.audioInfoToTrack(audioInfo, mp3Path);
    }

    // Download
    if (!audioInfo.audio_url) {
      throw new Error('Audio URL not available');
    }

    this.emit('download-started', { id: audioInfo.id, title: audioInfo.title });
    this.downloadProgress.set(audioInfo.id, 'Downloading...');
    this.emitState();

    try {
      // Download audio
      const response = await axios.get(audioInfo.audio_url, {
        responseType: 'arraybuffer',
        onDownloadProgress: (progressEvent) => {
          if (progressEvent.total) {
            const percentCompleted = Math.round((progressEvent.loaded * 100) / progressEvent.total);
            this.downloadProgress.set(audioInfo.id, `${percentCompleted}%`);
            this.emit('download-progress', { id: audioInfo.id, percent: percentCompleted });
            this.emitState();
          } else {
            this.downloadProgress.set(audioInfo.id, 'Downloading...');
            this.emitState();
          }
        },
      });

      await fs.writeFile(mp3Path, Buffer.from(response.data));

      // Also download image in background
      this.downloadImage(audioInfo).catch(err => {
        console.error('Failed to cache image:', err);
      });

      this.downloadProgress.delete(audioInfo.id);
      this.emit('download-completed', { id: audioInfo.id });
      this.emitState();

      return this.audioInfoToTrack(audioInfo, mp3Path);
    } catch (error: any) {
      this.downloadProgress.delete(audioInfo.id);
      this.emit('download-failed', { id: audioInfo.id, error: error.message });
      this.emitState();
      throw error;
    }
  }

  // ==================== State Functions ====================

  /**
   * Get current library state
   */
  getState(): LibraryState {
    // Add cached status and image path to each track
    const tracksWithCacheStatus = this.state.tracks.map(track => ({
      ...track,
      cached: this.isTrackCached(track.id),
      cachedImagePath: this.getCachedImagePath(track.id),
    }));

    return {
      ...this.state,
      tracks: tracksWithCacheStatus,
    };
  }


  /**
   * Get download progress
   */
  getDownloadProgress(): Map<string, string> {
    return new Map(this.downloadProgress);
  }

  /**
   * Check if track is cached
   */
  isTrackCached(trackId: string): boolean {
    const mp3Path = path.join(this.cacheDir, `${trackId}.mp3`);
    return existsSync(mp3Path);
  }

  /**
   * Get cached image path
   */
  getCachedImagePath(trackId: string): string | null {
    const imagePath = path.join(this.cacheDir, `${trackId}.jpg`);
    return existsSync(imagePath) ? imagePath : null;
  }

  /**
   * Convert AudioInfo to Track
   */
  private audioInfoToTrack(audioInfo: AudioInfo, mp3Path: string): Track {
    let duration = 180; // default 3 minutes
    if (audioInfo.duration) {
      if (typeof audioInfo.duration === 'string') {
        const seconds = parseFloat(audioInfo.duration);
        if (!isNaN(seconds)) {
          duration = seconds;
        }
      } else if (typeof audioInfo.duration === 'number') {
        duration = audioInfo.duration;
      }
    }

    return {
      id: audioInfo.id,
      title: audioInfo.title || 'Untitled',
      mp3Path,
      duration,
    };
  }

  /**
   * Emit library state change event
   */
  private emitState(): void {
    this.emit('state-changed', this.getState());
  }
}
