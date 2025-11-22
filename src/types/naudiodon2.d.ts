declare module 'naudiodon2' {
  export interface PortAudioDevice {
    id: number;
    name: string;
    maxOutputChannels: number;
  }

  export interface AudioIOOutOptions {
    channelCount: number;
    sampleFormat: number;
    sampleRate: number;
    deviceId: number;
    closeOnError: boolean;
  }

  export interface AudioIOOptions {
    outOptions: AudioIOOutOptions;
  }

  export class AudioIO {
    constructor(options: AudioIOOptions);
    start(): void;
    write(buffer: Buffer): boolean;
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
