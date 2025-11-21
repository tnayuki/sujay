declare module 'soundtouchjs' {
  export class FifoSampleBuffer {
    vector: Float32Array;
    position: number;
    startIndex: number;
    frameCount: number;
    endIndex: number;
    _vector: Float32Array;
    clear(): void;
    put(numFrames: number): void;
    receive(numFrames?: number): number;
    ensureCapacity(numFrames: number): void;
    ensureAdditionalCapacity(numFrames: number): void;
  }

  export class SoundTouch {
    constructor();
    inputBuffer: FifoSampleBuffer;
    outputBuffer: FifoSampleBuffer;
    tempo: number;
    rate: number;
    pitch: number;
    pitchSemitones: number;
    clear(): void;
    clone(): SoundTouch;
    process(): void;
  }

  export class RateTransposer {
    constructor();
    rate: number;
    clear(): void;
    clone(): RateTransposer;
    process(): void;
  }

  export class Stretch {
    constructor();
    tempo: number;
    clear(): void;
    clone(): Stretch;
    process(): void;
  }

  export class SimpleFilter {
    constructor(sourceSound: any, pipe: any, callback?: () => void);
    position: number;
    sourcePosition: number;
    extract(target: Float32Array, numFrames: number): number;
  }

  export class PitchShifter {
    constructor(context: AudioContext, buffer: AudioBuffer, bufferSize: number);
    tempo: number;
    pitch: number;
    pitchSemitones: number;
    rate: number;
    connect(toNode: AudioNode): void;
    disconnect(): void;
    on(eventName: string, cb: (detail: any) => void): void;
    off(eventName?: string): void;
  }
}
