/**
 * TimeStretcher - PCM buffer time-stretching using SoundTouch
 * Allows tempo changes while maintaining pitch
 */

import { SoundTouch, FifoSampleBuffer } from 'soundtouchjs';

interface InternalSoundTouch extends SoundTouch {
  inputBuffer: FifoSampleBuffer;
  outputBuffer: FifoSampleBuffer;
  stretch?: { inputChunkSize?: number };
  tempo: number;
  clear(): void;
  process(): void;
}

export class TimeStretcher {
  private soundtouch: InternalSoundTouch;
  private readonly CHANNELS: number = 2;
  private currentTempo = 1.0;
  // How many output frames we try to produce per call (passed from engine)
  // We keep a small reservoir so SoundTouch can produce smooth overlap.
  private reservoirTargetMultiplier = 2.0;

  constructor() {
    this.soundtouch = new SoundTouch() as InternalSoundTouch;
    this.soundtouch.tempo = 1.0;
  }

  /**
   * Process PCM buffer with time-stretching
   * @param pcmData Input PCM buffer (16-bit stereo)
   * @param position Current playback position in frames
   * @param tempo Tempo multiplier (1.0 = normal, <1.0 = slower, >1.0 = faster)
   * @param framesPerChunk Number of output frames to generate
   * @param output Output buffer to write to
   * @returns New position after processing
   */
  process(
    pcmData: Float32Array,
    position: number,
    tempo: number,
    framesPerChunk: number,
    output: Float32Array
  ): number {
    const totalFrames = Math.floor(pcmData.length / this.CHANNELS);

    // Update tempo if changed
    if (Math.abs(tempo - this.currentTempo) > 0.001) {
      this.soundtouch.tempo = tempo;
      this.currentTempo = tempo;
    }

    const inputBuffer = this.soundtouch.inputBuffer;
    const outputBuffer = this.soundtouch.outputBuffer;

    // Keep feeding input until we have enough output frames buffered.
    // Desired reservoir ensures overlap windows are satisfied.
    const desiredOutputReservoir = Math.ceil(framesPerChunk * this.reservoirTargetMultiplier);

    let fedFrames = 0;
    // Loop: feed blocks until we have enough buffered output or run out of input
    while (outputBuffer.frameCount < desiredOutputReservoir) {
      const remaining = totalFrames - Math.floor(position) - fedFrames;
      if (remaining <= 0) break;

      // Use stretch's inputChunkSize if available for optimal feeding size
      const stretch = this.soundtouch.stretch;
      const chunkSize = stretch && stretch.inputChunkSize ? stretch.inputChunkSize : Math.min(4096, remaining);
      const toFeed = Math.min(chunkSize, remaining);

      const endIndex = inputBuffer.endIndex;
      const requiredLength = endIndex + toFeed * this.CHANNELS;
      if (inputBuffer.vector.length < requiredLength) {
        const newVector = new Float32Array(requiredLength);
        newVector.set(inputBuffer.vector.subarray(0, endIndex));
        inputBuffer._vector = newVector; // direct internal resize
      }

      const startFrame = Math.floor(position) + fedFrames;
      for (let i = 0; i < toFeed; i++) {
        const srcFrame = startFrame + i;
        const srcBase = srcFrame * this.CHANNELS;
        if (srcFrame < totalFrames) {
          inputBuffer.vector[endIndex + i * 2] = pcmData[srcBase] ?? 0;
          inputBuffer.vector[endIndex + i * 2 + 1] = pcmData[srcBase + 1] ?? inputBuffer.vector[endIndex + i * 2];
        } else {
          inputBuffer.vector[endIndex + i * 2] = 0;
          inputBuffer.vector[endIndex + i * 2 + 1] = 0;
        }
      }
      inputBuffer.put(toFeed);
      fedFrames += toFeed;
      this.soundtouch.process();
    }

    // Read from output buffer
    const availableFrames = Math.min(outputBuffer.frameCount, framesPerChunk);
    const outputVector = outputBuffer.vector;
    const startIndex = outputBuffer.startIndex;

    for (let i = 0; i < framesPerChunk; i++) {
      const dstBase = i * this.CHANNELS;
      if (i < availableFrames) {
        const sampleL = Math.max(-1, Math.min(1, outputVector[startIndex + i * 2]));
        const sampleR = Math.max(-1, Math.min(1, outputVector[startIndex + i * 2 + 1]));
        output[dstBase] = sampleL;
        output[dstBase + 1] = sampleR;
      } else {
        output[dstBase] = 0;
        output[dstBase + 1] = 0;
      }
    }

    // Consume processed frames from output buffer
    if (availableFrames > 0) {
      outputBuffer.receive(availableFrames);
    }

    // Advance position ONLY by frames we actually fed into SoundTouch
    const newPosition = position + fedFrames;
    
    return newPosition;
  }

  /**
   * Clear internal buffers
   */
  clear(): void {
    this.soundtouch.clear();
  }
}
