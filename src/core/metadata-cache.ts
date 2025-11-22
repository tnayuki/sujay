/**
 * Metadata Cache - JSON-based caching for offline support
 */

import { promises as fs , existsSync } from 'fs';
import path from 'path';
import type { AudioInfo } from '../suno-api.js';
import type { Workspace } from '../types.js';

interface MetadataJson {
  workspaces: Workspace[];
  tracks: {
    [workspaceId: string]: AudioInfo[];
  };
  lastUpdated: number;
}

export class MetadataCache {
  private metadataPath: string;

  constructor(cacheDir: string) {
    this.metadataPath = path.join(cacheDir, 'metadata.json');
  }

  /**
   * Load metadata from JSON file
   */
  private async load(): Promise<MetadataJson> {
    if (!existsSync(this.metadataPath)) {
      return {
        workspaces: [],
        tracks: {},
        lastUpdated: 0,
      };
    }

    try {
      const data = await fs.readFile(this.metadataPath, 'utf-8');
      const parsed = JSON.parse(data);

      // Handle migration from old format (tracksByWorkspace -> tracks)
      if (parsed.tracksByWorkspace && !parsed.tracks) {
        parsed.tracks = parsed.tracksByWorkspace;
        delete parsed.tracksByWorkspace;
      }

      // Ensure tracks exists
      if (!parsed.tracks) {
        parsed.tracks = {};
      }

      return parsed;
    } catch (error) {
      console.error('Error loading metadata cache:', error);
      return {
        workspaces: [],
        tracks: {},
        lastUpdated: 0,
      };
    }
  }

  /**
   * Save metadata to JSON file
   */
  private async save(metadata: MetadataJson): Promise<void> {
    try {
      console.log('[MetadataCache] Saving to:', this.metadataPath);
      // Ensure directory exists
      await fs.mkdir(path.dirname(this.metadataPath), { recursive: true });
      await fs.writeFile(this.metadataPath, JSON.stringify(metadata, null, 2), 'utf-8');
      console.log('[MetadataCache] Saved successfully');
    } catch (error) {
      console.error('[MetadataCache] Error saving metadata cache:', error);
      throw error;
    }
  }

  /**
   * Get workspaces from cache
   */
  async getWorkspaces(): Promise<Workspace[] | null> {
    const metadata = await this.load();
    return metadata.workspaces.length > 0 ? metadata.workspaces : null;
  }

  /**
   * Save workspaces to cache
   */
  async saveWorkspaces(workspaces: Workspace[]): Promise<void> {
    const metadata = await this.load();
    metadata.workspaces = workspaces;
    metadata.lastUpdated = Date.now();
    await this.save(metadata);
  }

  /**
   * Get tracks for a workspace from cache
   */
  async getWorkspaceTracks(workspaceId: string | null): Promise<AudioInfo[] | null> {
    const metadata = await this.load();
    const key = workspaceId || 'default';
    return metadata.tracks[key] || null;
  }

  /**
   * Save tracks for a workspace to cache
   */
  async saveWorkspaceTracks(workspaceId: string | null, tracks: AudioInfo[]): Promise<void> {
    const metadata = await this.load();
    const key = workspaceId || 'default';
    metadata.tracks[key] = tracks;
    metadata.lastUpdated = Date.now();
    await this.save(metadata);
  }

  /**
   * Get last updated timestamp
   */
  async getLastUpdated(): Promise<number> {
    const metadata = await this.load();
    return metadata.lastUpdated;
  }

  /**
   * Clear all cached metadata
   */
  async clear(): Promise<void> {
    if (existsSync(this.metadataPath)) {
      await fs.unlink(this.metadataPath);
    }
  }
}
