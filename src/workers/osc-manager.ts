/**
 * OSC Manager - Sends BPM and track info via OSC
 */

// eslint-disable-next-line import/no-unresolved
import * as osc from 'node-osc';
import type { Track } from '../types';

export interface OSCConfig {
  host: string;
  port: number;
  enabled: boolean;
}

export class OSCManager {
  private client: osc.Client | null = null;
  private config: OSCConfig;

  constructor(config: OSCConfig = { host: '127.0.0.1', port: 9000, enabled: true }) {
    this.config = config;
    if (config.enabled) {
      this.connect();
    }
  }

  /**
   * Connect to OSC server
   */
  connect(): void {
    try {
      this.client = new osc.Client(this.config.host, this.config.port);
      console.log(`[OSC] Connected to ${this.config.host}:${this.config.port}`);
    } catch (error) {
      console.error('[OSC] Failed to connect:', error);
      this.client = null;
    }
  }

  /**
   * Disconnect OSC client
   */
  disconnect(): void {
    if (this.client) {
      this.client.close();
      this.client = null;
      console.log('[OSC] Disconnected');
    }
  }

  /**
   * Update OSC configuration
   */
  updateConfig(config: Partial<OSCConfig>): void {
    const oldConfig = { ...this.config };
    this.config = { ...this.config, ...config };

    // Reconnect if host or port changed
    if (
      this.config.enabled &&
      (oldConfig.host !== this.config.host || oldConfig.port !== this.config.port)
    ) {
      this.disconnect();
      this.connect();
    }

    // Connect if enabled, disconnect if disabled
    if (this.config.enabled && !oldConfig.enabled) {
      this.connect();
    } else if (!this.config.enabled && oldConfig.enabled) {
      this.disconnect();
    }
  }

  /**
   * Send master tempo (float)
   */
  sendMasterTempo(bpm: number): void {
    if (!this.client || !this.config.enabled) return;
    try {
      // Ensure float type by using Number() and adding decimal point if needed
      const floatBpm = Number(bpm);
      this.client.send('/sujay/tempo', floatBpm);
    } catch (error) {
      console.error('[OSC] Failed to send tempo:', error);
    }
  }

  /**
   * Send currently playing track title only
   */
  sendCurrentTrack(track: Track | null, deck: 'A' | 'B'): void {
    if (!this.client || !this.config.enabled) return;
    try {
      const title = track?.title ?? '';
      this.client.send(`/sujay/deck/${deck}/title`, title);
    } catch (error) {
      console.error('[OSC] Failed to send track title:', error);
    }
  }

}
