import Store from 'electron-store';
import type { AudioConfig, OSCConfig, RecordingConfig } from '../../types';

interface AppStoreSchema {
  osc: OSCConfig;
  audio: AudioConfig;
  recording: RecordingConfig;
}

interface AppStore {
  get(key: 'osc'): OSCConfig;
  get(key: 'audio'): AudioConfig;
  get(key: 'recording'): RecordingConfig;
  set(key: 'osc', value: OSCConfig): void;
  set(key: 'audio', value: AudioConfig): void;
  set(key: 'recording', value: RecordingConfig): void;
}

export interface AppSettingsStore {
  getOscConfig(): OSCConfig;
  setOscConfig(config: OSCConfig): void;
  getAudioConfig(): AudioConfig;
  setAudioConfig(config: AudioConfig): void;
  getRecordingConfig(): RecordingConfig;
  setRecordingConfig(config: RecordingConfig): void;
}

export function createAppSettingsStore(defaultRecordingConfig: RecordingConfig): AppSettingsStore {
  const raw = new Store<AppStoreSchema>({
    defaults: {
      osc: {
        enabled: false,
        host: '127.0.0.1',
        port: 9000,
      },
      audio: {
        mainChannels: [0, 1],
        cueChannels: [null, null],
      },
      recording: defaultRecordingConfig,
    },
    schema: {
      osc: {
        type: 'object',
        properties: {
          enabled: { type: 'boolean' },
          host: { type: 'string' },
          port: { type: 'number', minimum: 1, maximum: 65535 },
        },
        required: ['enabled', 'host', 'port'],
      },
      audio: {
        type: 'object',
        properties: {
          deviceId: { type: ['string', 'null'] },
          mainChannels: { type: 'array', items: { type: ['number', 'null'] }, minItems: 2, maxItems: 2 },
          cueChannels: { type: 'array', items: { type: ['number', 'null'] }, minItems: 2, maxItems: 2 },
        },
        required: ['mainChannels', 'cueChannels'],
      },
      recording: {
        type: 'object',
        properties: {
          directory: { type: 'string' },
          autoCreateDirectory: { type: 'boolean' },
          namingStrategy: { type: 'string', enum: ['timestamp', 'sequential'] },
          format: { type: 'string', enum: ['wav', 'ogg'] },
        },
        required: ['directory', 'autoCreateDirectory', 'namingStrategy', 'format'],
      },
    },
    migrations: {
      '>=0.0.0': (store) => {
        try {
          const rec = store.get('recording') as Partial<RecordingConfig> | undefined;
          if (!rec || typeof rec !== 'object') {
            store.set('recording', defaultRecordingConfig);
          } else if (rec.format !== 'wav' && rec.format !== 'ogg') {
            store.set('recording', { ...defaultRecordingConfig, ...rec, format: 'wav' });
          }
        } catch {
          store.set('recording', defaultRecordingConfig);
        }
      },
    },
  });

  const store = raw as unknown as AppStore;

  return {
    getOscConfig() {
      return store.get('osc');
    },
    setOscConfig(config) {
      store.set('osc', config);
    },
    getAudioConfig() {
      return store.get('audio');
    },
    setAudioConfig(config) {
      store.set('audio', config);
    },
    getRecordingConfig() {
      return store.get('recording');
    },
    setRecordingConfig(config) {
      store.set('recording', config);
    },
  };
}
