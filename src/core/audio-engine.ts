/**
 * Audio Engine - Handles playback, crossfade, and audio output
 */

import portAudio from 'naudiodon2';
import ffmpeg from 'fluent-ffmpeg';
import { EventEmitter } from 'events';
import type { Track, AudioEngineState } from '../types.js';

export class AudioEngine extends EventEmitter {
  private audioOutput: any = null;
  private playbackLoop: boolean = false;
  private deckA: Track | null = null;
  private deckB: Track | null = null;
  private deckAPosition: number = 0;
  private deckBPosition: number = 0;
  private deckAPlaying: boolean = false;
  private deckBPlaying: boolean = false;
  private crossfadeFrames: number = 0;
  private crossfadeDirection: 'AtoB' | 'BtoA' | null = null;
  private waveformSent: Set<string> = new Set();
  private activeWaveformGeneration: Set<string> = new Set();

  private manualCrossfaderPosition: number = 0;
  private isManualCrossfade: boolean = true;
  private isM4Device: boolean = false;

  private readonly SAMPLE_RATE = 44100;
  private readonly CHANNELS = 2;
  private readonly CROSSFADE_DURATION = 2;

  constructor() {
    super();
  }

  /**
   * Initialize audio output
   */
  async initialize(): Promise<void> {
    const devices = portAudio.getDevices();

    let deviceId = -1;
    const m4Device = devices.find((d: any) => d.name.includes('M4') && d.maxOutputChannels >= 4);

    if (m4Device) {
      deviceId = m4Device.id;
      this.isM4Device = true;
      console.log(`Using M4 audio device: ${m4Device.name} (ID: ${deviceId}), output to channels 3/4`);
    } else {
      console.log('M4 device not found, using default audio device');
    }

  this.audioOutput = new (portAudio as any).AudioIO({
      outOptions: {
        channelCount: this.isM4Device ? 4 : this.CHANNELS,
        sampleFormat: portAudio.SampleFormat16Bit,
        sampleRate: this.SAMPLE_RATE,
        deviceId,
        closeOnError: false,
      },
    });

    this.audioOutput.start();
    this.playbackLoop = true;
    this.startPlaybackLoop();
  }

  /**
   * Load track PCM data and start waveform generation
   */
  async loadTrackPCM(track: Track): Promise<Track> {
    return new Promise((resolve, reject) => {
      const chunks: Buffer[] = [];

      const command = ffmpeg(track.mp3Path)
        .format('s16le')
        .audioCodec('pcm_s16le')
        .audioFrequency(this.SAMPLE_RATE)
        .audioChannels(this.CHANNELS)
        .audioFilters('volume=1.0')
        .on('error', (err) => {
          reject(new Error(`ffmpeg error: ${err.message}`));
        })
        .on('end', () => {
          console.log(`[loadTrackPCM] FFmpeg completed for "${track.title}"]`);
          const pcmData = Buffer.concat(chunks);

          const trackWithoutWaveform = {
            ...track,
            pcmData,
            sampleRate: this.SAMPLE_RATE,
            channels: this.CHANNELS,
          };

          console.log('[loadTrackPCM] Resolving immediately (before waveform generation)');
          resolve(trackWithoutWaveform);

          console.log('[loadTrackPCM] Starting background waveform generation');
          this.generateAndSendWaveform(track.id, pcmData).catch((err) => {
            console.error(`Error generating waveform for track ${track.id}:`, err);
          });
        });

      const stream = command.pipe();
      stream.on('data', (chunk: Buffer) => {
        chunks.push(chunk);
      });
      stream.on('error', reject);
    });
  }

  private cancelWaveformGeneration(trackId: string): void {
    if (this.activeWaveformGeneration.has(trackId)) {
      console.log(`[Waveform] Canceling generation for track ${trackId.substring(0, 8)}`);
      this.activeWaveformGeneration.delete(trackId);
    }
  }

  private async generateAndSendWaveform(trackId: string, pcmData: Buffer): Promise<void> {
    this.activeWaveformGeneration.add(trackId);

    const startTime = Date.now();
    const bytesPerFrame = this.CHANNELS * 2;
    const totalFrames = Math.floor(pcmData.length / bytesPerFrame);
    const waveformData = new Float32Array(totalFrames);

    console.log(`[Waveform] Starting generation for track ${trackId.substring(0, 8)}, ${totalFrames} frames`);

    for (let i = 0; i < totalFrames; i++) {
      const byteOffset = i * bytesPerFrame;
      const sample = pcmData.readInt16LE(byteOffset);
      waveformData[i] = sample / 32768;
    }

    if (!this.activeWaveformGeneration.has(trackId)) {
      console.log(`[Waveform] Generation cancelled for track ${trackId.substring(0, 8)}`);
      return;
    }

    const genTime = Date.now() - startTime;
    console.log(`[Waveform] Generation completed in ${genTime}ms`);

    const CHUNK_SIZE = 50000;
    const totalChunks = Math.ceil(totalFrames / CHUNK_SIZE);

    console.log(`[Waveform] Sending ${totalChunks} chunks...`);
    const sendStartTime = Date.now();

    for (let i = 0; i < totalChunks; i++) {
      if (!this.activeWaveformGeneration.has(trackId)) {
        console.log(`[Waveform] Chunk sending cancelled for track ${trackId.substring(0, 8)} at chunk ${i}/${totalChunks}`);
        return;
      }

      const start = i * CHUNK_SIZE;
      const end = Math.min(start + CHUNK_SIZE, totalFrames);
      const chunk = Array.from(waveformData.slice(start, end));

      this.emit('waveform-chunk', {
        trackId,
        chunkIndex: i,
        totalChunks,
        chunk,
      });

      await new Promise((resolve) => setImmediate(resolve));
    }

    const sendTime = Date.now() - sendStartTime;
    const totalTime = Date.now() - startTime;
    console.log(`[Waveform] All chunks sent in ${sendTime}ms (total: ${totalTime}ms)`);

    this.emit('waveform-complete', {
      trackId,
      totalFrames,
    });

    this.activeWaveformGeneration.delete(trackId);
  }

  /**
   * Play a track with optional crossfade / target deck load
   */
  async play(inputTrack: Track, crossfade: boolean = false, targetDeck: 1 | 2 | null = null): Promise<void> {
    try {
      const newTrack = await this.loadTrackPCM(inputTrack);

      if (!newTrack.pcmData || !newTrack.sampleRate || !newTrack.channels) {
        throw new Error('Failed to load PCM data');
      }

      if ((this.deckAPlaying || this.deckBPlaying) && crossfade) {
        if (this.deckA && !this.deckB) {
          this.deckB = newTrack;
          this.deckBPosition = 0;
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'AtoB';
        } else if (this.deckB && !this.deckA) {
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'BtoA';
        } else if (this.deckA && this.deckB) {
          this.deckB = newTrack;
          this.deckBPosition = 0;
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'AtoB';
        } else {
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.deckAPlaying = true;
          this.crossfadeDirection = null;
        }
      } else {
        if (targetDeck === 2) {
          if (this.deckB) {
            this.cancelWaveformGeneration(this.deckB.id);
          }
          this.deckB = newTrack;
          this.deckBPosition = 0;
        } else {
          if (this.deckA) {
            this.cancelWaveformGeneration(this.deckA.id);
          }
          this.deckA = newTrack;
          this.deckAPosition = 0;
        }
        this.crossfadeDirection = null;
      }

      this.emitState();
    } catch (error) {
      console.error('Error in AudioEngine.play():', error);
      this.emit('error', error);
    }
  }

  /**
   * Stop playback on a deck
   */
  stop(deck: 1 | 2): void {
    if (deck === 1) {
      this.deckAPlaying = false;
    } else {
      this.deckBPlaying = false;
    }
    this.crossfadeFrames = 0;
    this.crossfadeDirection = null;

    if (this.audioOutput && !this.deckAPlaying && !this.deckBPlaying) {
      const silenceChannels = this.isM4Device ? 4 : this.CHANNELS;
      const silenceFrames = Math.floor(this.SAMPLE_RATE * 0.1);
      const silence = Buffer.alloc(silenceFrames * silenceChannels * 2);
      this.audioOutput.write(silence);
    }

    this.emitState();
  }

  /**
   * Seek within a deck
   */
  seek(deck: 1 | 2, position: number): void {
    position = Math.max(0, Math.min(1, position));

    if (deck === 1 && this.deckA && this.deckA.pcmData) {
      const totalFrames = Math.floor(this.deckA.pcmData.length / (this.CHANNELS * 2));
      this.deckAPosition = Math.floor(totalFrames * position);
    } else if (deck === 2 && this.deckB && this.deckB.pcmData) {
      const totalFrames = Math.floor(this.deckB.pcmData.length / (this.CHANNELS * 2));
      this.deckBPosition = Math.floor(totalFrames * position);
    }

    this.emitState();
  }

  setCrossfaderPosition(position: number): void {
    this.manualCrossfaderPosition = Math.max(0, Math.min(1, position));
    this.emitState();
  }

  startDeck(deck: 1 | 2): void {
    if (deck === 1 && this.deckA) {
      this.deckAPlaying = true;
    } else if (deck === 2 && this.deckB) {
      this.deckBPlaying = true;
    }
    this.emitState();
  }

  /**
   * Get current state
   */
  getState(): AudioEngineState {
    const sanitizeTrack = (track: Track | null): Track | null => {
      if (!track) return null;

      const shouldIncludeWaveform = track.waveformData && !this.waveformSent.has(track.id);
      if (shouldIncludeWaveform) {
        this.waveformSent.add(track.id);
      }

      const { pcmData, ...cleanTrack } = track;
      return {
        ...cleanTrack,
        waveformData: shouldIncludeWaveform ? cleanTrack.waveformData : undefined,
      } as Track;
    };

    const totalCrossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
    let crossfadeProgress = 0;

    if (this.crossfadeFrames > 0) {
      const progress = 1 - this.crossfadeFrames / totalCrossfadeFrames;
      if (this.crossfadeDirection === 'AtoB') {
        crossfadeProgress = progress;
      } else if (this.crossfadeDirection === 'BtoA') {
        crossfadeProgress = 1 - progress;
      }
    } else {
      crossfadeProgress = this.deckB && !this.deckA ? 1 : 0;
    }

    return {
      deckA: sanitizeTrack(this.deckA),
      deckB: sanitizeTrack(this.deckB),
      deckAPosition: this.deckAPosition / this.SAMPLE_RATE,
      deckBPosition: this.deckBPosition / this.SAMPLE_RATE,
      deckAPlaying: this.deckAPlaying,
      deckBPlaying: this.deckBPlaying,
      isPlaying: this.deckAPlaying || this.deckBPlaying,
      isCrossfading: this.crossfadeFrames > 0,
      crossfadeProgress,
      crossfaderPosition: this.manualCrossfaderPosition,
      currentTrack: sanitizeTrack(this.deckA),
      nextTrack: sanitizeTrack(this.deckB),
      position: this.deckAPosition / this.SAMPLE_RATE,
      nextPosition: this.deckBPosition / this.SAMPLE_RATE,
    };
  }

  async cleanup(): Promise<void> {
    this.playbackLoop = false;
    if (this.audioOutput) {
      await this.audioOutput.quit();
      this.audioOutput = null;
    }
  }

  private emitState(): void {
    this.emit('state-changed', this.getState());
  }

  private mixPCM(
    buffer1: Buffer,
    offset1: number,
    gain1: number,
    buffer2: Buffer,
    offset2: number,
    gain2: number,
    frames: number,
  ): Buffer {
    const bytesPerFrame = this.CHANNELS * 2;
    const outputSize = frames * bytesPerFrame;
    const output = Buffer.alloc(outputSize);

    for (let i = 0; i < frames; i++) {
      for (let ch = 0; ch < this.CHANNELS; ch++) {
        const byteOffset = i * bytesPerFrame + ch * 2;
        const sample1Offset = offset1 + byteOffset;
        const sample2Offset = offset2 + byteOffset;

        const sample1 = sample1Offset < buffer1.length ? buffer1.readInt16LE(sample1Offset) : 0;
        const sample2 = sample2Offset < buffer2.length ? buffer2.readInt16LE(sample2Offset) : 0;
        const mixed = Math.round(sample1 * gain1 + sample2 * gain2);
        const clamped = Math.max(-32768, Math.min(32767, mixed));

        output.writeInt16LE(clamped, byteOffset);
      }
    }

    return output;
  }

  private startPlaybackLoop(): void {
    const writeNextChunk = () => {
      if (!this.playbackLoop || !this.audioOutput) {
        return;
      }

      const bytesPerFrame = this.CHANNELS * 2;
      const framesPerChunk = Math.floor(this.SAMPLE_RATE * 0.05);

      // Check if either deck has ended
      if (this.deckA && this.deckAPlaying) {
        const deckATotalFrames = Math.floor(this.deckA.pcmData!.length / bytesPerFrame);
        if (this.deckAPosition >= deckATotalFrames) {
          this.deckAPlaying = false;
          this.deckAPosition = 0;
          this.emitState();
          this.emit('track-ended');
        }
      }
      if (this.deckB && this.deckBPlaying) {
        const deckBTotalFrames = Math.floor(this.deckB.pcmData!.length / bytesPerFrame);
        if (this.deckBPosition >= deckBTotalFrames) {
          this.deckBPlaying = false;
          this.deckBPosition = 0;
          this.emitState();
          this.emit('track-ended');
        }
      }

      // Calculate gains based on crossfader position and playing state
      const position = this.manualCrossfaderPosition;
      const deckAGain = (this.deckA && this.deckAPlaying) ? Math.cos((position * Math.PI) / 2) : 0;
      const deckBGain = (this.deckB && this.deckBPlaying) ? Math.sin((position * Math.PI) / 2) : 0;

      // Calculate how many frames to play
      let framesToPlay = framesPerChunk;
      if (this.deckA && this.deckAPlaying) {
        const deckATotalFrames = Math.floor(this.deckA.pcmData!.length / bytesPerFrame);
        const deckARemainingFrames = deckATotalFrames - this.deckAPosition;
        framesToPlay = Math.min(framesToPlay, deckARemainingFrames);
      }
      if (this.deckB && this.deckBPlaying) {
        const deckBTotalFrames = Math.floor(this.deckB.pcmData!.length / bytesPerFrame);
        const deckBRemainingFrames = deckBTotalFrames - this.deckBPosition;
        framesToPlay = Math.min(framesToPlay, deckBRemainingFrames);
      }

      // Mix the audio
      const chunk = this.mixPCM(
        this.deckA?.pcmData || Buffer.alloc(0),
        this.deckAPosition * bytesPerFrame,
        deckAGain,
        this.deckB?.pcmData || Buffer.alloc(0),
        this.deckBPosition * bytesPerFrame,
        deckBGain,
        framesToPlay,
      );

      // Update positions
      if (this.deckAPlaying) {
        this.deckAPosition += framesToPlay;
      }
      if (this.deckBPlaying) {
        this.deckBPosition += framesToPlay;
      }

      this.emitState();

      let outputChunk = chunk;
      if (this.isM4Device) {
        const framesInChunk = chunk.length / (this.CHANNELS * 2);
        const outputSize = framesInChunk * 4 * 2;
        outputChunk = Buffer.alloc(outputSize);

        for (let i = 0; i < framesInChunk; i++) {
          const leftSample = chunk.readInt16LE(i * 4 + 0);
          const rightSample = chunk.readInt16LE(i * 4 + 2);

          outputChunk.writeInt16LE(0, i * 8 + 0);
          outputChunk.writeInt16LE(0, i * 8 + 2);
          outputChunk.writeInt16LE(leftSample, i * 8 + 4);
          outputChunk.writeInt16LE(rightSample, i * 8 + 6);
        }
      }

      const canWrite = this.audioOutput.write(outputChunk);
      if (canWrite) {
        setTimeout(writeNextChunk, 0);
      } else {
        this.audioOutput.once('drain', writeNextChunk);
      }
    };

    writeNextChunk();
  }
}
