declare module 'naudiodon2' {
  export interface PortAudioDevice {
    id: number;
    name: string;
    maxInputChannels: number;
    maxOutputChannels: number;
    defaultSampleRate: number;
    defaultLowInputLatency?: number;
    defaultLowOutputLatency?: number;
    defaultHighInputLatency?: number;
    defaultHighOutputLatency?: number;
    hostAPIName?: string;
  }

  export interface AudioIOOutOptions {
    channelCount: number;
    sampleFormat: number;
    sampleRate: number;
    deviceId: number;
    closeOnError: boolean;
  }

  export interface AudioIOInOptions {
    channelCount: number;
    sampleFormat: number;
    sampleRate: number;
    deviceId: number;
    closeOnError: boolean;
  }

  export interface AudioIOOptions {
    inOptions?: AudioIOInOptions;
    outOptions: AudioIOOutOptions;
  }

  export class AudioIO {
    constructor(options: AudioIOOptions);
    start(): void;
    write(buffer: Buffer): boolean;
    on(event: 'data', listener: (buf: Buffer) => void): void;
    once(event: 'drain', listener: () => void): void;
    quit(): void | Promise<void>;
  }

  export const SampleFormatFloat32: number;
  export function getDevices(): PortAudioDevice[];

  const portAudio: {
    AudioIO: typeof AudioIO;
    SampleFormatFloat32: number;
    getDevices: () => PortAudioDevice[];
  };
  export default portAudio;
}
