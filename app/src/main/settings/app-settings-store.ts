import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import type { AudioConfig, OSCConfig, RecordingConfig } from '../../types';

interface AppSettingsData {
  osc: OSCConfig;
  audio: AudioConfig;
  recording: RecordingConfig;
}

interface StoredSettingsData {
  osc?: Partial<OSCConfig>;
  audio?: Partial<AudioConfig>;
  recording?: Partial<RecordingConfig>;
}

export interface AppSettingsStore {
  getOscConfig(): OSCConfig;
  setOscConfig(config: OSCConfig): void;
  getAudioConfig(): AudioConfig;
  setAudioConfig(config: AudioConfig): void;
  getRecordingConfig(): RecordingConfig;
  setRecordingConfig(config: RecordingConfig): void;
}

const defaultOscConfig: OSCConfig = {
  enabled: false,
  host: '127.0.0.1',
  port: 9000,
};

const defaultAudioConfig: AudioConfig = {
  mainChannels: [0, 1],
  cueChannels: [null, null],
};

function normalizeAudioChannels(channels: unknown, fallback: [number | null, number | null]): [number | null, number | null] {
  if (!Array.isArray(channels) || channels.length !== 2) {
    return fallback;
  }

  const [left, right] = channels;
  const normalizeChannel = (value: unknown): number | null => {
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value;
    }
    return null;
  };

  return [normalizeChannel(left), normalizeChannel(right)];
}

function normalizeSettingsData(input: StoredSettingsData | null | undefined, defaultRecordingConfig: RecordingConfig): AppSettingsData {
  const osc = input?.osc ?? {};
  const audio = input?.audio ?? {};
  const recording = input?.recording ?? {};

  return {
    osc: {
      enabled: typeof osc.enabled === 'boolean' ? osc.enabled : defaultOscConfig.enabled,
      host: typeof osc.host === 'string' && osc.host.trim().length > 0 ? osc.host : defaultOscConfig.host,
      port: typeof osc.port === 'number' && Number.isFinite(osc.port) ? osc.port : defaultOscConfig.port,
    },
    audio: {
      deviceId: typeof audio.deviceId === 'string' && audio.deviceId.trim().length > 0 ? audio.deviceId : undefined,
      mainChannels: normalizeAudioChannels(audio.mainChannels, defaultAudioConfig.mainChannels),
      cueChannels: normalizeAudioChannels(audio.cueChannels, defaultAudioConfig.cueChannels),
    },
    recording: {
      directory: typeof recording.directory === 'string' && recording.directory.trim().length > 0
        ? recording.directory
        : defaultRecordingConfig.directory,
      autoCreateDirectory: typeof recording.autoCreateDirectory === 'boolean'
        ? recording.autoCreateDirectory
        : defaultRecordingConfig.autoCreateDirectory,
      namingStrategy: recording.namingStrategy === 'sequential' ? 'sequential' : 'timestamp',
      format: recording.format === 'ogg' ? 'ogg' : 'wav',
    },
  };
}

function loadSettings(storageFilePath: string, defaultRecordingConfig: RecordingConfig): AppSettingsData {
  try {
    const raw = readFileSync(storageFilePath, 'utf8');
    const parsed = JSON.parse(raw) as StoredSettingsData;
    return normalizeSettingsData(parsed, defaultRecordingConfig);
  } catch {
    return normalizeSettingsData(null, defaultRecordingConfig);
  }
}

function persistSettings(storageFilePath: string, state: AppSettingsData) {
  const directory = path.dirname(storageFilePath);
  mkdirSync(directory, { recursive: true });
  const tempPath = `${storageFilePath}.tmp`;
  writeFileSync(tempPath, `${JSON.stringify(state, null, 2)}\n`, 'utf8');
  renameSync(tempPath, storageFilePath);
}

export function createAppSettingsStore(storageFilePath: string, defaultRecordingConfig: RecordingConfig): AppSettingsStore {
  let state = loadSettings(storageFilePath, defaultRecordingConfig);
  persistSettings(storageFilePath, state);

  return {
    getOscConfig() {
      return state.osc;
    },
    setOscConfig(config) {
      state = { ...state, osc: config };
      persistSettings(storageFilePath, state);
    },
    getAudioConfig() {
      return state.audio;
    },
    setAudioConfig(config) {
      state = { ...state, audio: config };
      persistSettings(storageFilePath, state);
    },
    getRecordingConfig() {
      return state.recording;
    },
    setRecordingConfig(config) {
      state = { ...state, recording: config };
      persistSettings(storageFilePath, state);
    },
  };
}
