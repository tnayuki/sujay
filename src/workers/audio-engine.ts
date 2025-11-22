/**
 * Audio Engine - Handles playback, crossfade, and audio output
 */

import portAudio, { AudioIO, PortAudioDevice } from 'naudiodon2';
import { EventEmitter } from 'events';
import type { Track, AudioEngineState, AudioLevelState, OSCConfig, AudioConfig } from '../types';
import { BPMDetector } from './bpm-detector';
import { OSCManager } from './osc-manager';
import { TimeStretcher } from './time-stretcher';

type DecodeResult = { pcmData: Float32Array; float32Mono: Float32Array; bpm: number | undefined };

export class AudioEngine extends EventEmitter {
  private readonly decodeTrack: (track: Track) => Promise<DecodeResult>;
  private audioOutput: AudioIO | null = null;
  private playbackLoop = false;
  private deckA: Track | null = null;
  private deckB: Track | null = null;
  private deckAPosition = 0;
  private deckBPosition = 0;
  private deckAPlaying = false;
  private deckBPlaying = false;
  private crossfadeFrames = 0;
  private crossfadeDirection: 'AtoB' | 'BtoA' | null = null;
  private waveformSent: Set<string> = new Set();
  private activeWaveformGeneration: Set<string> = new Set();

  private manualCrossfaderPosition = 0;
  private isManualCrossfade = true;
  private isM4Device = false;
  private audioConfig: AudioConfig = { mainChannels: [0, 1], cueChannels: [null, null] };
  private outputChannelCount = 2;
  private cueEnabled = false;

  private masterTempo = 130;
  private deckARate = 1.0;
  private deckBRate = 1.0;
  private deckALevel = 0;
  private deckBLevel = 0;
  private deckACueEnabled = false;
  private deckBCueEnabled = false;

  // Pre-allocated buffers for playback loop
  private resampleBufferA: Float32Array = new Float32Array(0);
  private resampleBufferB: Float32Array = new Float32Array(0);
  private mixBuffer: Float32Array = new Float32Array(0);
  private outputFloatBuffer: Float32Array = new Float32Array(0);
  private lastEmitTime = 0;
  private readonly EMIT_INTERVAL_MS = 16; // Emit state max 60 times per second for smooth playback
  private lastDeckAId: string | null = null;
  private lastDeckBId: string | null = null;
  private lastEmittedDeckAPosition = 0;
  private lastEmittedDeckBPosition = 0;
  private lastEmittedMasterTempo: number | null = null;
  private shouldEmitPosition = false; // Flag to emit position on next state emission
  private isSeekOperation = false; // Flag to indicate position change is from seek
  private oscManager: OSCManager;
  private timeStretcherA: TimeStretcher;
  private timeStretcherB: TimeStretcher;
  private deviceMonitorInterval: NodeJS.Timeout | null = null;
  private lastDeviceCount = 0;

  private readonly SAMPLE_RATE: number = 44100;
  private readonly CHANNELS: number = 2;
  private readonly CROSSFADE_DURATION = 2;
  private readonly MIN_RATE = 0.5;
  private readonly MAX_RATE = 2.0;

  constructor(decodeTrack: (track: Track) => Promise<DecodeResult>) {
    super();
    this.decodeTrack = decodeTrack;
    this.oscManager = new OSCManager();
    this.timeStretcherA = new TimeStretcher();
    this.timeStretcherB = new TimeStretcher();
  }

  applyAudioConfig(config: AudioConfig): void {
    this.audioConfig = { ...this.audioConfig, ...config };
    this.updateCueRoutingState();
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

    // Test Float32 support for each device
    const testFloat32Support = (deviceId: number): boolean => {
      try {
        const test = new AudioIO({
          outOptions: {
            channelCount: 2,
            sampleFormat: portAudio.SampleFormatFloat32,
            sampleRate: this.SAMPLE_RATE,
            deviceId,
            closeOnError: false,
          },
        });
        test.quit();
        return true;
      } catch (err) {
        console.warn(`[Audio] Device ${deviceId} does not support Float32:`, err);
        return false;
      }
    };

    // Pick an output-capable device (>= 2 channels) with Float32 support
    const outputCapable = devices.filter((d: PortAudioDevice) => 
      (d.maxOutputChannels ?? 0) >= 2 && testFloat32Support(d.id)
    );
    let selected: PortAudioDevice | null = null;

    if (this.audioConfig.deviceId !== undefined) {
      const dev = devices.find((d: PortAudioDevice) => d.id === this.audioConfig.deviceId);
      if (dev && (dev.maxOutputChannels ?? 0) >= 2 && testFloat32Support(dev.id)) {
        selected = dev;
      } else {
        console.warn('[Audio] Selected device is not output-capable (>=2 + Float32). Falling back. id=', this.audioConfig.deviceId);
      }
    }

    if (!selected) {
      // Prefer M4 if available (>=4ch), otherwise first output-capable device
      const m4 = outputCapable.find((d: PortAudioDevice) => String(d.name || '').includes('M4') && d.maxOutputChannels >= 4) || null;
      selected = m4 || outputCapable[0] || null;
      if (selected) {
        if (m4) {
          this.isM4Device = true;
          console.log(`Using M4 audio device: ${selected.name} (ID: ${selected.id})`);
          // M4 default mapping: MAIN -> ch3/4, CUE -> ch1/2
          this.audioConfig.mainChannels = [2, 3];
          this.audioConfig.cueChannels = [0, 1];
        } else {
          console.log(`Using output device: ${selected.name} (ID: ${selected.id})`);
        }
      }
    }

    if (!selected) {
      throw new Error('No output-capable audio device found (needs >=2 output channels with Float32 support)');
    }

    // Determine required output channel count based on mapping
    const indices = [
      this.audioConfig.mainChannels[0],
      this.audioConfig.mainChannels[1],
      this.audioConfig.cueChannels[0],
      this.audioConfig.cueChannels[1],
    ].filter((v): v is number => v !== null && v !== undefined);
    const maxIndex = indices.length ? Math.max(...indices) : -1;
    const required = Math.max(2, maxIndex + 1);

    // Clamp to device capability (do NOT coerce device max up)
    const maxOut = selected.maxOutputChannels ?? 2;
    this.outputChannelCount = Math.min(required, maxOut);

    // If mapping exceeds available channels, remap safely
    const mappingExceeds = indices.some((i) => i >= this.outputChannelCount);
    if (mappingExceeds) {
      console.warn('[Audio] Channel mapping exceeds device capability. Remapping to stereo MAIN only.');
      this.audioConfig.mainChannels = [0, 1];
      this.audioConfig.cueChannels = [null, null];
      this.updateCueRoutingState();
    }

    this.audioOutput = new AudioIO({
      outOptions: {
        channelCount: this.outputChannelCount,
        sampleFormat: portAudio.SampleFormatFloat32,
        sampleRate: this.SAMPLE_RATE,
        deviceId: selected.id,
        closeOnError: false,
      },
    });

    this.audioOutput.start();
    this.playbackLoop = true;
    this.startPlaybackLoop();
    this.startDeviceMonitoring();
    this.updateCueRoutingState();
  }

  /**
   * Monitor device changes via polling
   */
  private startDeviceMonitoring(): void {
    this.lastDeviceCount = portAudio.getDevices().length;
    this.deviceMonitorInterval = setInterval(() => {
      const currentCount = portAudio.getDevices().length;
      if (currentCount !== this.lastDeviceCount) {
        console.log(`[Audio] Device count changed: ${this.lastDeviceCount} -> ${currentCount}`);
        this.lastDeviceCount = currentCount;
        this.emit('device-changed', { deviceCount: currentCount });
      }
    }, 2000); // Check every 2 seconds
  }

  private stopDeviceMonitoring(): void {
    if (this.deviceMonitorInterval) {
      clearInterval(this.deviceMonitorInterval);
      this.deviceMonitorInterval = null;
    }
  }

  /**
   * Load track PCM data and start waveform generation
   */
  async loadTrackPCM(track: Track): Promise<Track> {
    const loadStartTime = Date.now();
    console.log(`[loadTrackPCM] Starting load for "${track.title}"`);
    
    const { pcmData, float32Mono, bpm: detectedBpm } = await this.decodeTrack(track);

    const bpm = detectedBpm;
    if (bpm) {
      console.log(`[loadTrackPCM] Detected BPM: ${bpm}`);
    } else {
      console.log('[loadTrackPCM] BPM detection failed');
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
    console.log(`[loadTrackPCM] Total load time (mpg123-decoder): ${totalLoadTime}ms`);
    console.log('[loadTrackPCM] Resolving immediately (before waveform generation)');
    
    this.generateAndSendWaveform(track.id, pcmData).catch((err) => {
      console.error(`Error generating waveform for track ${track.id}:`, err);
    });

    return trackWithoutWaveform;
  }

  private cancelWaveformGeneration(trackId: string): void {
    if (this.activeWaveformGeneration.has(trackId)) {
      console.log(`[Waveform] Canceling generation for track ${trackId.substring(0, 8)}`);
      this.activeWaveformGeneration.delete(trackId);
    }
  }

  private async generateAndSendWaveform(trackId: string, pcmData: Float32Array): Promise<void> {
    this.activeWaveformGeneration.add(trackId);

    const startTime = Date.now();
    const totalFrames = this.getFrameCount(pcmData);

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
      const chunk = new Array(end - start);
      for (let j = 0; j < chunk.length; j++) {
        const frameIndex = start + j;
        chunk[j] = this.getMonoSample(pcmData, frameIndex);
      }

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
  async play(inputTrack: Track, crossfade = false, targetDeck: 1 | 2 | null = null): Promise<void> {
    try {
      // If track already has PCM data, use it directly. Otherwise, load it.
      let newTrack: Track;
      if (inputTrack.pcmData && inputTrack.float32Mono) {
        console.log(`[play] Track "${inputTrack.title}" already has PCM data, using it directly`);
        newTrack = inputTrack;
        // If BPM is not set, try to detect it from existing float32Mono data
        if (!newTrack.bpm && newTrack.float32Mono) {
          console.log(`[play] BPM not set for "${newTrack.title}", detecting from existing PCM data (${newTrack.float32Mono.length} samples)`);
          const detectedBPM = BPMDetector.detect(newTrack.float32Mono, this.SAMPLE_RATE);
          if (detectedBPM) {
            console.log(`[play] Detected BPM: ${detectedBPM}`);
            newTrack = { ...newTrack, bpm: detectedBPM };
          } else {
            console.log('[play] BPM detection failed');
          }
        } else if (newTrack.bpm) {
          console.log(`[play] Track already has BPM: ${newTrack.bpm}`);
        }
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
          this.shouldEmitPosition = true;
          this.isSeekOperation = true;
          this.deckBRate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'B');
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'AtoB';
        } else if (this.deckB && !this.deckA) {
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.shouldEmitPosition = true;
          this.isSeekOperation = true;
          this.deckARate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'A');
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'BtoA';
        } else if (this.deckA && this.deckB) {
          this.deckB = newTrack;
          this.deckBPosition = 0;
          this.shouldEmitPosition = true;
          this.isSeekOperation = true;
          this.deckBRate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'B');
          this.crossfadeFrames = this.SAMPLE_RATE * this.CROSSFADE_DURATION;
          this.crossfadeDirection = 'AtoB';
        } else {
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.shouldEmitPosition = true;
          this.isSeekOperation = true;
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
          this.shouldEmitPosition = true;
          this.isSeekOperation = true;
          this.deckBRate = this.calculatePlaybackRate(newTrack);
          this.oscManager.sendCurrentTrack(newTrack, 'B');
        } else {
          if (this.deckA) {
            this.cancelWaveformGeneration(this.deckA.id);
          }
          this.deckA = newTrack;
          this.deckAPosition = 0;
          this.shouldEmitPosition = true;
          this.isSeekOperation = true;
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
      const silenceChannels = this.outputChannelCount;
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
      const totalFrames = this.getFrameCount(this.deckA.pcmData);
      this.deckAPosition = Math.floor(totalFrames * position);
    } else if (deck === 2 && this.deckB && this.deckB.pcmData) {
      const totalFrames = this.getFrameCount(this.deckB.pcmData);
      this.deckBPosition = Math.floor(totalFrames * position);
    }

    this.shouldEmitPosition = true;
    this.isSeekOperation = true;
    this.emitState(true);
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
    console.log(`[AudioEngine] setMasterTempo: ${bpm} (was ${this.masterTempo})`);
    this.masterTempo = bpm;
    if (this.deckA) {
      const oldRateA = this.deckARate;
      this.deckARate = this.calculatePlaybackRate(this.deckA);
      console.log(`[AudioEngine] Deck A rate: ${oldRateA} -> ${this.deckARate} (track BPM: ${this.deckA.bpm || 'not detected'})`);
      if (!this.deckA.bpm) {
        console.warn('[AudioEngine] Deck A has no BPM, tempo change will have no effect. Reload track to detect BPM.');
      }
    }
    if (this.deckB) {
      const oldRateB = this.deckBRate;
      this.deckBRate = this.calculatePlaybackRate(this.deckB);
      console.log(`[AudioEngine] Deck B rate: ${oldRateB} -> ${this.deckBRate} (track BPM: ${this.deckB.bpm || 'not detected'})`);
      if (!this.deckB.bpm) {
        console.warn('[AudioEngine] Deck B has no BPM, tempo change will have no effect. Reload track to detect BPM.');
      }
    }
    this.oscManager.sendMasterTempo(bpm);
    this.emitState();
  }

  setDeckCueEnabled(deck: 1 | 2, enabled: boolean): void {
    if (deck === 1) {
      this.deckACueEnabled = enabled;
    } else {
      this.deckBCueEnabled = enabled;
    }
    this.updateCueRoutingState();
    this.emitState(true);
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

  private updateCueRoutingState(): void {
    const [cueLeft, cueRight] = this.audioConfig.cueChannels;
    const hasCueOutputs = (cueLeft ?? null) !== null || (cueRight ?? null) !== null;
    this.cueEnabled = hasCueOutputs && (this.deckACueEnabled || this.deckBCueEnabled);
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

      const cleanTrack = { ...track };
      delete cleanTrack.pcmData;
      return {
        ...cleanTrack,
        waveformData: shouldIncludeWaveform ? cleanTrack.waveformData : undefined,
      } as Omit<Track, 'pcmData'>;
    };

    // Check if track info changed
    const deckAChanged = this.deckA?.id !== this.lastDeckAId;
    const deckBChanged = this.deckB?.id !== this.lastDeckBId;

    if (deckAChanged) this.lastDeckAId = this.deckA?.id || null;
    if (deckBChanged) this.lastDeckBId = this.deckB?.id || null;

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

    // Always emit positions during playback or when forced
    const deckAPositionSeconds = this.deckAPosition / this.SAMPLE_RATE;
    const deckBPositionSeconds = this.deckBPosition / this.SAMPLE_RATE;
    const forceEmit = this.shouldEmitPosition;
    const deckAPositionChanged = forceEmit || this.deckAPlaying;
    const deckBPositionChanged = forceEmit || this.deckBPlaying;
    const isSeek = this.isSeekOperation;

    if (deckAPositionChanged) {
      this.lastEmittedDeckAPosition = deckAPositionSeconds;
    }
    if (deckBPositionChanged) {
      this.lastEmittedDeckBPosition = deckBPositionSeconds;
    }
    if (forceEmit) {
      this.shouldEmitPosition = false;
    }
    if (isSeek) {
      this.isSeekOperation = false;
    }

    const masterTempoChanged = this.lastEmittedMasterTempo !== this.masterTempo;
    if (masterTempoChanged) {
      this.lastEmittedMasterTempo = this.masterTempo;
    }

    return {
      deckA: deckAChanged ? sanitizeTrack(this.deckA) : undefined,
      deckB: deckBChanged ? sanitizeTrack(this.deckB) : undefined,
      deckAPosition: deckAPositionChanged ? deckAPositionSeconds : undefined,
      deckBPosition: deckBPositionChanged ? deckBPositionSeconds : undefined,
      isSeek: (deckAPositionChanged || deckBPositionChanged) ? isSeek : undefined,
      deckAPlaying: this.deckAPlaying,
      deckBPlaying: this.deckBPlaying,
      isPlaying: this.deckAPlaying || this.deckBPlaying,
      isCrossfading: this.crossfadeFrames > 0,
      crossfadeProgress,
      crossfaderPosition: this.manualCrossfaderPosition,
      masterTempo: masterTempoChanged ? this.masterTempo : undefined,
      deckALevel: this.deckALevel,
      deckBLevel: this.deckBLevel,
      deckACueEnabled: this.deckACueEnabled,
      deckBCueEnabled: this.deckBCueEnabled,
      currentTrack: deckAChanged ? sanitizeTrack(this.deckA) : undefined,
      nextTrack: deckBChanged ? sanitizeTrack(this.deckB) : undefined,
      position: deckAPositionChanged ? deckAPositionSeconds : undefined,
      nextPosition: deckBPositionChanged ? deckBPositionSeconds : undefined,
    };
  }

  async cleanup(): Promise<void> {
    this.playbackLoop = false;
    this.stopDeviceMonitoring();
    if (this.audioOutput) {
      await this.audioOutput.quit();
      this.audioOutput = null;
    }
  }

  private getLevelState(): AudioLevelState {
    return {
      deckALevel: this.deckALevel,
      deckBLevel: this.deckBLevel,
    };
  }

  private emitState(force = false): void {
    const now = Date.now();
    if (force || now - this.lastEmitTime >= this.EMIT_INTERVAL_MS) {
      this.emit('state-changed', this.getState());
      this.lastEmitTime = now;
    }
  }

  private emitLevelState(): void {
    this.emit('level-state', this.getLevelState());
  }

  private mixPCM(
    buffer1: Float32Array,
    offset1: number,
    gain1: number,
    buffer2: Float32Array,
    offset2: number,
    gain2: number,
    frames: number,
    output: Float32Array,
  ): void {
    const channels = this.CHANNELS;
    const samplesPerFrame = channels;

    for (let frame = 0; frame < frames; frame++) {
      for (let ch = 0; ch < channels; ch++) {
        const sampleOffset = frame * samplesPerFrame + ch;
        const sample1Index = offset1 + sampleOffset;
        const sample2Index = offset2 + sampleOffset;
        const sample1 = sample1Index < buffer1.length ? buffer1[sample1Index] : 0;
        const sample2 = sample2Index < buffer2.length ? buffer2[sample2Index] : 0;
        output[sampleOffset] = sample1 * gain1 + sample2 * gain2;
      }
    }
  }

  private startPlaybackLoop(): void {
    const framesPerChunk = 2048;
    const samplesPerChunk = framesPerChunk * this.CHANNELS;

    const [mL, mR] = this.audioConfig.mainChannels;
    const [cL, cR] = this.audioConfig.cueChannels;
    const mappingRequired = (
      this.outputChannelCount !== this.CHANNELS ||
      mL === null || mR === null ||
      mL === mR ||
      mL !== 0 || mR !== 1 ||
      cL !== null || cR !== null
    );

    this.resampleBufferA = this.ensureFloatCapacity(this.resampleBufferA, samplesPerChunk);
    this.resampleBufferB = this.ensureFloatCapacity(this.resampleBufferB, samplesPerChunk);
    this.mixBuffer = this.ensureFloatCapacity(this.mixBuffer, samplesPerChunk);
    if (mappingRequired) {
      this.outputFloatBuffer = this.ensureFloatCapacity(this.outputFloatBuffer, framesPerChunk * this.outputChannelCount);
    }

    const writeNextChunk = (): void => {
      if (!this.playbackLoop || !this.audioOutput) {
        return;
      }

      try {
        if (this.deckAPlaying && this.deckA) {
          this.deckAPosition = this.timeStretcherA.process(
            this.deckA.pcmData,
            this.deckAPosition,
            this.deckARate,
            framesPerChunk,
            this.resampleBufferA
          );
        } else if (this.deckA) {
          this.resampleBufferA.fill(0);
        }

        if (this.deckBPlaying && this.deckB) {
          this.deckBPosition = this.timeStretcherB.process(
            this.deckB.pcmData,
            this.deckBPosition,
            this.deckBRate,
            framesPerChunk,
            this.resampleBufferB
          );
        } else if (this.deckB) {
          this.resampleBufferB.fill(0);
        }

        const position = this.manualCrossfaderPosition;
        const deckAGain = this.deckAPlaying ? Math.cos((position * Math.PI) / 2) : 0;
        const deckBGain = this.deckBPlaying ? Math.sin((position * Math.PI) / 2) : 0;

        this.deckALevel = this.calculateRMS(this.resampleBufferA, framesPerChunk);
        this.deckBLevel = this.calculateRMS(this.resampleBufferB, framesPerChunk);

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

        let stateChanged = false;
        if (this.deckA && this.deckAPlaying) {
          const totalFrames = this.getFrameCount(this.deckA.pcmData);
          if (this.deckAPosition >= totalFrames) {
            this.deckAPlaying = false;
            this.deckAPosition = 0;
            this.emit('track-ended');
            stateChanged = true;
          }
        }
        if (this.deckB && this.deckBPlaying) {
          const totalFrames = this.getFrameCount(this.deckB.pcmData);
          if (this.deckBPosition >= totalFrames) {
            this.deckBPlaying = false;
            this.deckBPosition = 0;
            this.emit('track-ended');
            stateChanged = true;
          }
        }

        this.emitState(stateChanged);
        this.emitLevelState();
      } catch (err) {
        console.error('Error during playback processing:', err);
        this.mixBuffer.fill(0);
      }

      let floatChunk = this.mixBuffer;
      let channelCount = this.CHANNELS;

      if (mappingRequired) {
        const mappedBuffer = this.outputFloatBuffer;
        mappedBuffer.fill(0);

        const [mainLch, mainRch] = this.audioConfig.mainChannels;
        const [cueLch, cueRch] = this.audioConfig.cueChannels;

        for (let frame = 0; frame < framesPerChunk; frame++) {
          const mixBase = frame * this.CHANNELS;
          const outBase = frame * this.outputChannelCount;
          const mainLeft = this.mixBuffer[mixBase] ?? 0;
          const mainRight = this.mixBuffer[mixBase + 1] ?? mainLeft;
          const monoMain = (mainLeft + mainRight) * 0.5;

          if (mainLch !== null && mainRch !== null && mainLch !== mainRch) {
            mappedBuffer[outBase + mainLch] = mainLeft;
            mappedBuffer[outBase + mainRch] = mainRight;
          } else if (mainLch !== null) {
            mappedBuffer[outBase + mainLch] = monoMain;
          } else if (mainRch !== null) {
            mappedBuffer[outBase + mainRch] = monoMain;
          }

          if (this.cueEnabled) {
            let cueLeft = 0;
            let cueRight = 0;
            let cueSources = 0;

            if (this.deckACueEnabled) {
              const aL = this.resampleBufferA[mixBase] ?? 0;
              const aR = this.resampleBufferA[mixBase + 1] ?? aL;
              cueLeft += aL;
              cueRight += aR;
              cueSources++;
            }

            if (this.deckBCueEnabled) {
              const bL = this.resampleBufferB[mixBase] ?? 0;
              const bR = this.resampleBufferB[mixBase + 1] ?? bL;
              cueLeft += bL;
              cueRight += bR;
              cueSources++;
            }

            if (cueSources > 0) {
              const normalization = 1 / cueSources;
              cueLeft = Math.max(-1, Math.min(1, cueLeft * normalization));
              cueRight = Math.max(-1, Math.min(1, cueRight * normalization));
              const monoCue = (cueLeft + cueRight) * 0.5;

              if (cueLch !== null && cueRch !== null && cueLch !== cueRch) {
                mappedBuffer[outBase + cueLch] = cueLeft;
                mappedBuffer[outBase + cueRch] = cueRight;
              } else if (cueLch !== null) {
                mappedBuffer[outBase + cueLch] = monoCue;
              } else if (cueRch !== null) {
                mappedBuffer[outBase + cueRch] = monoCue;
              }
            }
          }
        }

        floatChunk = mappedBuffer;
        channelCount = this.outputChannelCount;
      }

      const outputChunk = this.getFloat32Buffer(floatChunk, channelCount, framesPerChunk);
      const canWrite = this.audioOutput.write(outputChunk);
      if (canWrite) {
        setImmediate(writeNextChunk);
      } else {
        this.audioOutput.once('drain', writeNextChunk);
      }
    };

    writeNextChunk();
  }

  private simpleResamplePCM(pcmData: Float32Array, position: number, rate: number, framesPerChunk: number, output: Float32Array): void {
    const totalFrames = this.getFrameCount(pcmData);
    const samplesPerFrame = this.CHANNELS;

    for (let i = 0; i < framesPerChunk; i++) {
      const srcIdx = Math.floor(position + i * rate);
      const dstBase = i * samplesPerFrame;
      if (srcIdx < totalFrames) {
        const srcBase = srcIdx * samplesPerFrame;
        for (let ch = 0; ch < this.CHANNELS; ch++) {
          output[dstBase + ch] = pcmData[srcBase + ch] ?? 0;
        }
      } else {
        for (let ch = 0; ch < this.CHANNELS; ch++) {
          output[dstBase + ch] = 0;
        }
      }
    }
  }

  private calculateRMS(buffer: Float32Array, frames: number): number {
    const samplesPerFrame = this.CHANNELS;
    const availableFrames = Math.min(frames, Math.floor(buffer.length / samplesPerFrame));
    let sumSquares = 0;
    let sampleCount = 0;

    for (let i = 0; i < availableFrames; i++) {
      const base = i * samplesPerFrame;
      for (let ch = 0; ch < this.CHANNELS; ch++) {
        const sample = buffer[base + ch] ?? 0;
        sumSquares += sample * sample;
        sampleCount++;
      }
    }

    if (sampleCount === 0) {
      return 0;
    }
    return Math.sqrt(sumSquares / sampleCount);
  }

  private bufferToFloat32Array(buffer: Buffer): Float32Array {
    if (buffer.length % 4 !== 0) {
      throw new Error('Float32 PCM buffer length must be a multiple of 4 bytes');
    }
    const view = buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.length);
    return new Float32Array(view);
  }

  private float32StereoToMono(buffer: Float32Array): Float32Array {
    const frames = this.getFrameCount(buffer);
    const mono = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
      mono[i] = this.getMonoSample(buffer, i);
    }
    return mono;
  }

  private getMonoSample(buffer: Float32Array, frameIndex: number): number {
    const base = frameIndex * this.CHANNELS;
    const left = buffer[base] ?? 0;
    if (this.CHANNELS === 1) {
      return left;
    }
    const right = buffer[base + 1] ?? left;
    return (left + right) * 0.5;
  }

  private ensureFloatCapacity(buffer: Float32Array, requiredSamples: number): Float32Array {
    if (buffer.length === requiredSamples) {
      return buffer;
    }
    return new Float32Array(requiredSamples);
  }

  private getFloat32Buffer(source: Float32Array, channelCount: number, frames: number): Buffer {
    const requiredSamples = frames * channelCount;
    const clipped = new Float32Array(requiredSamples);
    for (let i = 0; i < requiredSamples; i++) {
      clipped[i] = Math.max(-1, Math.min(1, source[i] ?? 0));
    }
    return Buffer.from(clipped.buffer, clipped.byteOffset, clipped.byteLength);
  }

  private getFrameCount(pcmData: Float32Array): number {
    return Math.floor(pcmData.length / this.CHANNELS);
  }
}
