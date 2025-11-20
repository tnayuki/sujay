/**
 * Audio Engine - Handles playback, crossfade, and audio output
 */

import portAudio from 'naudiodon2';
import ffmpeg from 'fluent-ffmpeg';
import { EventEmitter } from 'events';
import type { Track, AudioEngineState, OSCConfig } from '../types.js';
import { BPMDetector } from './bpm-detector.js';
import { OSCManager } from './osc-manager.js';

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

  private masterTempo: number = 130;
  private deckARate: number = 1.0;
  private deckBRate: number = 1.0;

  // Pre-allocated buffers for playback loop
  private resampleBufferA: Buffer = Buffer.alloc(0);
  private resampleBufferB: Buffer = Buffer.alloc(0);
  private mixBuffer: Buffer = Buffer.alloc(0);
  private outputBuffer: Buffer = Buffer.alloc(0);
  private lastEmitTime: number = 0;
  private readonly EMIT_INTERVAL_MS = 100; // Emit state max 10 times per second
  private oscManager: OSCManager;

  private readonly SAMPLE_RATE = 44100;
  private readonly CHANNELS = 2;
  private readonly CROSSFADE_DURATION = 2;
  private readonly MIN_RATE = 0.5;
  private readonly MAX_RATE = 2.0;

  constructor() {
    super();
    this.oscManager = new OSCManager();
  }

  /**
   * Update OSC configuration
   */
  updateOSCConfig(config: OSCConfig): void {
    this.oscManager.updateConfig(config);
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
    const loadStartTime = Date.now();
    console.log(`[loadTrackPCM] Starting load for "${track.title}"`);
    
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
          const decodeTime = Date.now() - loadStartTime;
          console.log(`[loadTrackPCM] FFmpeg decode completed in ${decodeTime}ms for "${track.title}"`);
          const pcmData = Buffer.concat(chunks);
          const float32Mono = this.bufferToFloat32Mono(pcmData);

          let bpm = track.bpm;
          if (!bpm) {
            console.log(`[loadTrackPCM] Detecting BPM for "${track.title}"`);
            bpm = BPMDetector.detect(float32Mono, this.SAMPLE_RATE) ?? undefined;
            if (bpm) {
              console.log(`[loadTrackPCM] Detected BPM: ${bpm}`);
            } else {
              console.log('[loadTrackPCM] BPM detection failed');
            }
          }

          const trackWithoutWaveform = {
            ...track,
            pcmData,
            sampleRate: this.SAMPLE_RATE,
            channels: this.CHANNELS,
            bpm,
            float32Mono,
          };

          const totalLoadTime = Date.now() - loadStartTime;
          console.log(`[loadTrackPCM] Total load time: ${totalLoadTime}ms (decode: ${decodeTime}ms, BPM: ${totalLoadTime - decodeTime}ms)`);
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

    console.log(`[Waveform] Starting generation for track ${trackId.substring(0, 8)}, ${totalFrames} frames`);

    const CHUNK_SIZE = 50000;
    const totalChunks = Math.ceil(totalFrames / CHUNK_SIZE);

    console.log(`[Waveform] Generating and sending ${totalChunks} chunks...`);
    const sendStartTime = Date.now();

    for (let i = 0; i < totalChunks; i++) {
      if (!this.activeWaveformGeneration.has(trackId)) {
        console.log(`[Waveform] Chunk sending cancelled for track ${trackId.substring(0, 8)} at chunk ${i}/${totalChunks}`);
        return;
      }

      const start = i * CHUNK_SIZE;
      const end = Math.min(start + CHUNK_SIZE, totalFrames);
      
      // Generate chunk on-the-fly to save memory
      const chunkData = new Float32Array(end - start);
      for (let j = 0; j < chunkData.length; j++) {
        const byteOffset = (start + j) * bytesPerFrame;
        const sample = pcmData.readInt16LE(byteOffset);
        chunkData[j] = sample / 32768;
      }
      const chunk = Array.from(chunkData);

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
      // If track already has PCM data, use it directly. Otherwise, load it.
      let newTrack: Track;
      if (inputTrack.pcmData && inputTrack.float32Mono) {
        console.log(`[play] Track "${inputTrack.title}" already has PCM data, using it directly`);
        newTrack = inputTrack;
      } else {
        console.log(`[play] Track "${inputTrack.title}" needs PCM loading`);
        newTrack = await this.loadTrackPCM(inputTrack);
      }

      if (!newTrack.pcmData || !newTrack.sampleRate || !newTrack.channels) {
        throw new Error('Failed to load PCM data');
      }

      if ((this.deckAPlaying || this.deckBPlaying) && crossfade) {
        if (this.deckA && !this.deckB) {
          this.deckB = newTrack;
          this.deckBPosition = 0;
          this.deckBRate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'B');
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'AtoB';
        } else if (this.deckB && !this.deckA) {
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.deckARate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'A');
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'BtoA';
        } else if (this.deckA && this.deckB) {
          this.deckB = newTrack;
          this.deckBPosition = 0;
          this.deckBRate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'B');
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'AtoB';
        } else {
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.deckARate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'A');
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
          this.deckBRate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'B');
        } else {
          if (this.deckA) {
            this.cancelWaveformGeneration(this.deckA.id);
          }
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.deckARate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'A');
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

  /**
   * Set master tempo in BPM
   */
  setMasterTempo(bpm: number): void {
    if (bpm <= 0 || bpm > 300) return;
    this.masterTempo = bpm;
    if (this.deckA) {
      this.deckARate = this.calculatePlaybackRate(this.deckA);
    }
    if (this.deckB) {
      this.deckBRate = this.calculatePlaybackRate(this.deckB);
    }
    this.oscManager.sendMasterTempo(bpm);
    this.emitState();
  }

  /**
   * Calculate playback rate for a track based on master tempo
   */
  private calculatePlaybackRate(track: Track): number {
    if (!track.bpm || track.bpm <= 0) {
      return 1.0;
    }
    const rate = this.masterTempo / track.bpm;
    return Math.max(this.MIN_RATE, Math.min(this.MAX_RATE, rate));
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
      masterTempo: this.masterTempo,
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

  private emitState(force: boolean = false): void {
    const now = Date.now();
    if (force || now - this.lastEmitTime >= this.EMIT_INTERVAL_MS) {
      this.emit('state-changed', this.getState());
      this.lastEmitTime = now;
    }
  }

  private mixPCM(
    buffer1: Buffer,
    offset1: number,
    gain1: number,
    buffer2: Buffer,
    offset2: number,
    gain2: number,
    frames: number,
    output: Buffer,
  ): void {
    const bytesPerFrame = this.CHANNELS * 2;

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
  }

  private startPlaybackLoop(): void {
    const framesPerChunk = Math.floor(this.SAMPLE_RATE * 0.05);
    const bytesPerFrame = this.CHANNELS * 2;
    const chunkSize = framesPerChunk * bytesPerFrame;

    // Pre-allocate buffers
    this.resampleBufferA = Buffer.alloc(chunkSize);
    this.resampleBufferB = Buffer.alloc(chunkSize);
    this.mixBuffer = Buffer.alloc(chunkSize);
    if (this.isM4Device) {
      this.outputBuffer = Buffer.alloc(framesPerChunk * 4 * 2);
    }

    const writeNextChunk = (): void => {
      if (!this.playbackLoop || !this.audioOutput) {
        return;
      }

      try {
        // Resample only playing decks
        if (this.deckAPlaying && this.deckA) {
          this.simpleResamplePCM(this.deckA.pcmData, this.deckAPosition, this.deckARate, framesPerChunk, this.resampleBufferA);
        } else if (this.deckA) {
          this.resampleBufferA.fill(0);
        }

        if (this.deckBPlaying && this.deckB) {
          this.simpleResamplePCM(this.deckB.pcmData, this.deckBPosition, this.deckBRate, framesPerChunk, this.resampleBufferB);
        } else if (this.deckB) {
          this.resampleBufferB.fill(0);
        }

        // Calculate gains based on crossfader position
        const position = this.manualCrossfaderPosition;
        const deckAGain = this.deckAPlaying ? Math.cos((position * Math.PI) / 2) : 0;
        const deckBGain = this.deckBPlaying ? Math.sin((position * Math.PI) / 2) : 0;

        // Mix both decks (reusing buffer)
        this.mixPCM(
          this.resampleBufferA,
          0,
          deckAGain,
          this.resampleBufferB,
          0,
          deckBGain,
          framesPerChunk,
          this.mixBuffer,
        );

        // Update positions and check for track end
        let stateChanged = false;
        if (this.deckA && this.deckAPlaying) {
          this.deckAPosition += Math.floor(framesPerChunk * this.deckARate);
          const totalFrames = Math.floor(this.deckA.pcmData.length / bytesPerFrame);
          if (this.deckAPosition >= totalFrames) {
            this.deckAPlaying = false;
            this.deckAPosition = 0;
            this.emit('track-ended');
            stateChanged = true;
          }
        }
        if (this.deckB && this.deckBPlaying) {
          this.deckBPosition += Math.floor(framesPerChunk * this.deckBRate);
          const totalFrames = Math.floor(this.deckB.pcmData.length / bytesPerFrame);
          if (this.deckBPosition >= totalFrames) {
            this.deckBPlaying = false;
            this.deckBPosition = 0;
            this.emit('track-ended');
            stateChanged = true;
          }
        }
        
        // Emit state only if changed or at regular intervals
        this.emitState(stateChanged);
      } catch (err) {
        console.error('Error during playback processing:', err);
        this.mixBuffer.fill(0);
      }

      let outputChunk = this.mixBuffer;
      if (this.isM4Device) {
        // Reuse pre-allocated output buffer
        for (let i = 0; i < framesPerChunk; i++) {
          const leftSample = this.mixBuffer.readInt16LE(i * 4 + 0);
          const rightSample = this.mixBuffer.readInt16LE(i * 4 + 2);

          this.outputBuffer.writeInt16LE(0, i * 8 + 0);
          this.outputBuffer.writeInt16LE(0, i * 8 + 2);
          this.outputBuffer.writeInt16LE(leftSample, i * 8 + 4);
          this.outputBuffer.writeInt16LE(rightSample, i * 8 + 6);
        }
        outputChunk = this.outputBuffer;
      }

      const canWrite = this.audioOutput.write(outputChunk);
      if (canWrite) {
        // Small delay to prevent busy loop, audio buffer will queue
        setImmediate(writeNextChunk);
      } else {
        this.audioOutput.once('drain', writeNextChunk);
      }
    };

    writeNextChunk();
  }

  /**
   * Simple PCM resampling (pitch shift by rate)
   * Writes to the provided output buffer (in-place)
   */
  private simpleResamplePCM(pcmData: Buffer, position: number, rate: number, framesPerChunk: number, output: Buffer): void {
    const bytesPerFrame = this.CHANNELS * 2;
    const totalFrames = Math.floor(pcmData.length / bytesPerFrame);
    
    for (let i = 0; i < framesPerChunk; i++) {
      const srcIdx = Math.floor(position + i * rate);
      if (srcIdx < totalFrames) {
        const srcOffset = srcIdx * bytesPerFrame;
        for (let ch = 0; ch < this.CHANNELS; ch++) {
          output.writeInt16LE(pcmData.readInt16LE(srcOffset + ch * 2), i * bytesPerFrame + ch * 2);
        }
      } else {
        // End of track: write silence
        for (let ch = 0; ch < this.CHANNELS; ch++) {
          output.writeInt16LE(0, i * bytesPerFrame + ch * 2);
        }
      }
    }
  }

  /**
   * Convert stereo PCM buffer to mono Float32Array for BPM detection
   */
  private bufferToFloat32Mono(buffer: Buffer): Float32Array {
    const bytesPerFrame = this.CHANNELS * 2;
    const totalFrames = Math.floor(buffer.length / bytesPerFrame);
    const output = new Float32Array(totalFrames);

    for (let i = 0; i < totalFrames; i++) {
      const byteOffset = i * bytesPerFrame;
      let sample = buffer.readInt16LE(byteOffset);
      if (this.CHANNELS === 2) {
        const rightSample = buffer.readInt16LE(byteOffset + 2);
        sample = Math.floor((sample + rightSample) / 2);
      }
      output[i] = sample / 32768.0;
    }

    return output;
  }
}
