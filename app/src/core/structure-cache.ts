/**
 * Track Structure Cache - JSON-based caching for track structure analysis
 */

import { promises as fs, existsSync } from 'fs';
import path from 'path';
import type { TrackStructure } from '../types.js';

interface StructureCacheJson {
  structures: {
    [trackId: string]: TrackStructure;
  };
  lastUpdated: number;
}

export class StructureCache {
  private cachePath: string;

  constructor(cacheDir: string) {
    this.cachePath = path.join(cacheDir, 'track-structures.json');
  }

  /**
   * Load structure cache from JSON file
   */
  private async load(): Promise<StructureCacheJson> {
    if (!existsSync(this.cachePath)) {
      return {
        structures: {},
        lastUpdated: 0,
      };
    }

    try {
      const data = await fs.readFile(this.cachePath, 'utf-8');
      return JSON.parse(data);
    } catch (error) {
      console.error('Error loading structure cache:', error);
      return {
        structures: {},
        lastUpdated: 0,
      };
    }
  }

  /**
   * Save structure cache to JSON file
   */
  private async save(cache: StructureCacheJson): Promise<void> {
    try {
      console.log('[StructureCache] Saving to:', this.cachePath);
      // Ensure directory exists
      await fs.mkdir(path.dirname(this.cachePath), { recursive: true });
      await fs.writeFile(this.cachePath, JSON.stringify(cache, null, 2), 'utf-8');
      console.log('[StructureCache] Saved successfully');
    } catch (error) {
      console.error('[StructureCache] Error saving structure cache:', error);
      throw error;
    }
  }

  /**
   * Get structure for a track from cache
   */
  async getStructure(trackId: string): Promise<TrackStructure | null> {
    const cache = await this.load();
    return cache.structures[trackId] || null;
  }

  /**
   * Save structure for a track to cache
   */
  async saveStructure(trackId: string, structure: TrackStructure): Promise<void> {
    const cache = await this.load();
    cache.structures[trackId] = structure;
    cache.lastUpdated = Date.now();
    await this.save(cache);
  }

  /**
   * Get all structures from cache
   */
  async getAllStructures(): Promise<{ [trackId: string]: TrackStructure }> {
    const cache = await this.load();
    return cache.structures;
  }

  /**
   * Clear all cached structures
   */
  async clear(): Promise<void> {
    if (existsSync(this.cachePath)) {
      await fs.unlink(this.cachePath);
    }
  }

  /**
   * Delete structure for a specific track
   */
  async deleteStructure(trackId: string): Promise<void> {
    const cache = await this.load();
    delete cache.structures[trackId];
    cache.lastUpdated = Date.now();
    await this.save(cache);
  }
}
