/**
 * Audio Engine - Simple single-track playback
 */

import portAudio from 'naudiodon2';
import ffmpeg from 'fluent-ffmpeg';
import { EventEmitter } from 'events';
import type { Track, AudioEngineState } from '../types.js';

export class AudioEngine extends EventEmitter {
  private audioOutput: any = null;
  private playbackLoop: boolean = false;
  private currentTrack: Track | null = null;
  private position: number = 0; // Current position in frames
  private isPlaying: boolean = false;

  private readonly SAMPLE_RATE = 44100;
  private readonly CHANNELS = 2;

  constructor() {
    super();
  }

  /**
   * Initialize audio output
   */
  async initialize(): Promise<void> {
    // Initialize audio output with default device
    this.audioOutput = new portAudio.AudioIO({
      outOptions: {
        channelCount: this.CHANNELS,
        sampleFormat: portAudio.SampleFormat16Bit,
        sampleRate: this.SAMPLE_RATE,
        deviceId: -1, // Default device
        closeOnError: false,
      },
    });

    // Start audio output
    this.audioOutput.start();
    this.playbackLoop = true;
    this.startPlaybackLoop();
  }

  /**
   * Load track PCM data
   */
  async loadTrackPCM(track: Track): Promise<Track> {
    return new Promise((resolve, reject) => {
      const chunks: Buffer[] = [];

      const command = ffmpeg(track.mp3Path)
        .format('s16le')
        .audioCodec('pcm_s16le')
        .audioFrequency(this.SAMPLE_RATE)
        .audioChannels(this.CHANNELS)
        .on('error', (err) => {
          reject(new Error(`ffmpeg error: ${err.message}`));
        })
        .on('end', () => {
          const pcmData = Buffer.concat(chunks);
          resolve({
            ...track,
            pcmData,
            sampleRate: this.SAMPLE_RATE,
            channels: this.CHANNELS,
          });
        });

      const stream = command.pipe();
      stream.on('data', (chunk: Buffer) => {
        chunks.push(chunk);
      });
      stream.on('error', reject);
    });
  }

  /**
   * Play a track
   */
  async play(inputTrack: Track): Promise<void> {
    try {
      const newTrack = await this.loadTrackPCM(inputTrack);

      if (!newTrack.pcmData || !newTrack.sampleRate || !newTrack.channels) {
        throw new Error('Failed to load PCM data');
      }

      this.currentTrack = newTrack;
      this.position = 0;
      this.isPlaying = true;

      this.emitState();
    } catch (error) {
      console.error('Error in AudioEngine.play():', error);
      this.emit('error', error);
    }
  }

  /**
   * Stop playback
   */
  stop(): void {
    this.isPlaying = false;

    // Flush audio buffer with silence to prevent pops/clicks
    if (this.audioOutput) {
      const silenceFrames = Math.floor(this.SAMPLE_RATE * 0.1); // 100ms of silence
      const silence = Buffer.alloc(silenceFrames * this.CHANNELS * 2);
      this.audioOutput.write(silence);
    }

    this.emitState();
  }

  /**
   * Seek to a position (0-1)
   */
  seek(position: number): void {
    position = Math.max(0, Math.min(1, position));

    if (this.currentTrack && this.currentTrack.pcmData) {
      const totalFrames = Math.floor(this.currentTrack.pcmData.length / (this.CHANNELS * 2));
      this.position = Math.floor(totalFrames * position);
    }

    this.emitState();
  }

  /**
   * Get current state
   */
  getState(): AudioEngineState {
    const sanitizeTrack = (track: Track | null): Track | null => {
      if (!track) return null;
      const { pcmData, ...cleanTrack } = track;
      return cleanTrack as Track;
    };

    return {
      currentTrack: sanitizeTrack(this.currentTrack),
      position: this.currentTrack ? this.position / this.SAMPLE_RATE : 0,
      isPlaying: this.isPlaying,
      // For backward compatibility
      deckA: sanitizeTrack(this.currentTrack),
      deckB: null,
      deckAPosition: this.currentTrack ? this.position / this.SAMPLE_RATE : 0,
      deckBPosition: 0,
      deckAPlaying: this.isPlaying,
      deckBPlaying: false,
      isCrossfading: false,
      crossfadeProgress: 0,
      crossfaderPosition: 0,
      nextTrack: null,
      nextPosition: 0,
    };
  }

  /**
   * Cleanup
   */
  async cleanup(): Promise<void> {
    this.playbackLoop = false;
    if (this.audioOutput) {
      await this.audioOutput.quit();
      this.audioOutput = null;
    }
  }

  /**
   * Emit state change event
   */
  private emitState(): void {
    this.emit('state-changed', this.getState());
  }

  /**
   * Playback loop
   */
  private startPlaybackLoop(): void {
    const writeNextChunk = () => {
      if (!this.playbackLoop || !this.audioOutput) {
        return;
      }

      const bytesPerFrame = this.CHANNELS * 2;
      const framesPerChunk = Math.floor(this.SAMPLE_RATE * 0.05); // 50ms chunks

      if (!this.isPlaying || !this.currentTrack || !this.currentTrack.pcmData) {
        // Write silence and continue loop
        const silence = Buffer.alloc(framesPerChunk * this.CHANNELS * 2);
        this.audioOutput.write(silence);
        setTimeout(writeNextChunk, 100);
        return;
      }

      const byteOffset = this.position * bytesPerFrame;
      const totalFrames = Math.floor(this.currentTrack.pcmData.length / bytesPerFrame);

      // Check if track finished
      if (this.position >= totalFrames) {
        this.isPlaying = false;
        this.position = 0;
        this.emitState();
        this.emit('track-ended');
        setTimeout(writeNextChunk, 10);
        return;
      }

      const remainingFrames = totalFrames - this.position;
      const framesToPlay = Math.min(framesPerChunk, remainingFrames);
      const chunk = this.currentTrack.pcmData.subarray(byteOffset, byteOffset + framesToPlay * bytesPerFrame);

      this.position += framesToPlay;

      // Emit state on every chunk (every ~50ms)
      this.emitState();

      // Write to audio output
      const canWrite = this.audioOutput.write(chunk);
      if (canWrite) {
        setTimeout(writeNextChunk, 0);
      } else {
        this.audioOutput.once('drain', writeNextChunk);
      }
    };

    writeNextChunk();
  }
}
