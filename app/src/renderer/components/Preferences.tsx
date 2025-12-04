import React, { useState, useEffect } from 'react';
import type { OSCConfig, AudioConfig, AudioDevice, RecordingConfig, SunoConfig } from '../../types';
import './Preferences.css';

type PreferencesTab = 'audio' | 'recording' | 'osc' | 'suno';

interface PreferencesProps {
  onClose: () => void;
}

const Preferences: React.FC<PreferencesProps> = ({ onClose }) => {
  const [activeTab, setActiveTab] = useState<PreferencesTab>('audio');
  const [oscConfig, setOscConfig] = useState<OSCConfig>({
    enabled: false,
    host: '127.0.0.1',
    port: 9000,
  });

  const [audioConfig, setAudioConfig] = useState<AudioConfig>({
    mainChannels: [0, 1],
    cueChannels: [null, null],
  });

  const [audioDevices, setAudioDevices] = useState<AudioDevice[]>([]);

  const [tempConfig, setTempConfig] = useState<OSCConfig>(oscConfig);
  const [tempAudioConfig, setTempAudioConfig] = useState<AudioConfig>(audioConfig);
  const [recordingConfig, setRecordingConfig] = useState<RecordingConfig | null>(null);
  const [tempRecordingConfig, setTempRecordingConfig] = useState<RecordingConfig | null>(null);
  const [sunoConfig, setSunoConfig] = useState<SunoConfig>({ cookie: '' });
  const [tempSunoConfig, setTempSunoConfig] = useState<SunoConfig>({ cookie: '' });

  useEffect(() => {
    let mounted = true;

    const loadPreferences = async () => {
      try {
        const [osc, devices, audio, recording, suno] = await Promise.all([
          window.electronAPI.oscGetConfig(),
          window.electronAPI.audioGetDevices(),
          window.electronAPI.audioGetConfig(),
          window.electronAPI.recordingGetConfig(),
          window.electronAPI.sunoGetConfig(),
        ]);

        if (!mounted) {
          return;
        }

        setOscConfig(osc);
        setTempConfig(osc);
        setAudioDevices(devices);
        
        // Check if saved deviceId exists in current device list
        const savedDevice = audio.deviceId ? devices.find((d) => d.name === audio.deviceId) : null;
        
        // If device not found, clear the deviceId (fallback to System Default)
        const effectiveAudio = savedDevice ? audio : { ...audio, deviceId: undefined };
        
        setAudioConfig(effectiveAudio);
        setTempAudioConfig(effectiveAudio);
        setRecordingConfig(recording);
        setTempRecordingConfig(recording);
        setSunoConfig(suno);
        setTempSunoConfig(suno);
      } catch (error) {
        console.error('Failed to load preferences', error);
      }
    };

    loadPreferences();

    return () => {
      mounted = false;
    };
  }, []);

  const handleSave = async () => {
    try {
      const tasks: Promise<unknown>[] = [
        window.electronAPI.oscUpdateConfig(tempConfig),
        window.electronAPI.audioUpdateConfig(tempAudioConfig),
      ];
      if (tempRecordingConfig) {
        tasks.push(window.electronAPI.recordingUpdateConfig(tempRecordingConfig));
      }
      tasks.push(window.electronAPI.sunoUpdateConfig(tempSunoConfig));
      await Promise.all(tasks);
      setOscConfig(tempConfig);
      setAudioConfig(tempAudioConfig);
      if (tempRecordingConfig) {
        setRecordingConfig(tempRecordingConfig);
      }
      setSunoConfig(tempSunoConfig);
      onClose();
    } catch (error) {
      console.error('Failed to save preferences', error);
    }
  };

  const handleCancel = () => {
    setTempConfig(oscConfig);
    setTempAudioConfig(audioConfig);
    setTempRecordingConfig(recordingConfig);
    setTempSunoConfig(sunoConfig);
    onClose();
  };
  const handleSunoCookieChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setTempSunoConfig({ cookie: e.target.value });
  };

  const handleHostChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setTempConfig({ ...tempConfig, host: e.target.value });
  };

  const handlePortChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const port = parseInt(e.target.value, 10);
    if (!isNaN(port) && port > 0 && port <= 65535) {
      setTempConfig({ ...tempConfig, port });
    }
  };

  const handleEnabledChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setTempConfig({ ...tempConfig, enabled: e.target.checked });
  };

  const handleRecordingDirectoryChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (!tempRecordingConfig) {
      return;
    }
    setTempRecordingConfig({ ...tempRecordingConfig, directory: e.target.value });
  };

  const handleRecordingFormatChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    if (!tempRecordingConfig) {
      return;
    }
    setTempRecordingConfig({ ...tempRecordingConfig, format: e.target.value as 'wav' | 'ogg' });
  };

  const handleDeviceChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const deviceId = e.target.value;
    const device = deviceId ? audioDevices.find((d) => d.name === deviceId) : null;

    const channelLimit = device ? device.maxOutputChannels : 2;
    const clampPair = (pair: [number | null, number | null]): [number | null, number | null] => [
      pair[0] !== null && pair[0] >= channelLimit ? null : pair[0],
      pair[1] !== null && pair[1] >= channelLimit ? null : pair[1],
    ];

    setTempAudioConfig({
      ...tempAudioConfig,
      deviceId: deviceId || undefined,
      mainChannels: clampPair(tempAudioConfig.mainChannels),
      cueChannels: clampPair(tempAudioConfig.cueChannels),
    });
  };

  const handleChannelChange = (type: 'main' | 'cue', side: 'left' | 'right', value: number | null) => {
    // Build next config for the changed pair
    const key = type === 'main' ? 'mainChannels' : 'cueChannels';
    const channels = tempAudioConfig[key];
    const newPair: [number | null, number | null] = [
      side === 'left' ? value : channels[0],
      side === 'right' ? value : channels[1]
    ];

    // Start from current, then apply pair
    const next: AudioConfig = {
      ...tempAudioConfig,
      [key]: newPair
    } as AudioConfig;

    // Enforce uniqueness: if a value is selected, clear it from the other three slots
    const slots: Array<{ k: 'mainChannels' | 'cueChannels'; idx: 0 | 1 }> = [
      { k: 'mainChannels', idx: 0 },
      { k: 'mainChannels', idx: 1 },
      { k: 'cueChannels', idx: 0 },
      { k: 'cueChannels', idx: 1 },
    ];
    const changedSlotKey = key;
    const changedIdx: 0 | 1 = side === 'left' ? 0 : 1;
    const changedVal = value;
    if (changedVal !== null) {
      for (const s of slots) {
        if (!(s.k === changedSlotKey && s.idx === changedIdx)) {
          const v = next[s.k][s.idx];
          if (v === changedVal) {
            const arr = [...next[s.k]] as [number | null, number | null];
            arr[s.idx] = null;
            next[s.k] = arr;
          }
        }
      }
    }

    setTempAudioConfig(next);
  };

  // Get the currently selected device based on tempAudioConfig.deviceId
  // This ensures we always use the device from the current config, not stale state
  const currentDevice = tempAudioConfig.deviceId 
    ? audioDevices.find((d) => d.name === tempAudioConfig.deviceId) 
    : null;
  
  // Get max channels from current device, or 2 for System Default
  const effectiveMaxChannels = currentDevice?.maxOutputChannels ?? 2;
  const channelOptions = Array.from({ length: effectiveMaxChannels }, (_, i) => i);

  const renderChannelSelector = (
    configKey: 'main' | 'cue',
    label: string,
  ) => {
    const pair = configKey === 'main' ? tempAudioConfig.mainChannels : tempAudioConfig.cueChannels;

    return (
    <div className="preference-item channel-card">
      <span className="label-text">{label}</span>
      <div className="channel-controls">
        <div className="channel-pair">
          <span>L</span>
          <select
            value={pair[0] ?? -1}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10);
              handleChannelChange(configKey, 'left', v === -1 ? null : v);
            }}
          >
            <option value={-1}>-</option>
            {channelOptions.map((channel) => (
              <option key={channel} value={channel}>
                {channel + 1}
              </option>
            ))}
          </select>
        </div>
        <div className="channel-pair">
          <span>R</span>
          <select
            value={pair[1] ?? -1}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10);
              handleChannelChange(configKey, 'right', v === -1 ? null : v);
            }}
          >
            <option value={-1}>-</option>
            {channelOptions.map((channel) => (
              <option key={channel} value={channel}>
                {channel + 1}
              </option>
            ))}
          </select>
        </div>
      </div>
    </div>
    );
  };

  return (
    <div className="preferences-window">
      <header className="preferences-header">
        <div>
          <h1>Preferences</h1>
        </div>
        <button className="close-button" onClick={handleCancel} aria-label="Close preferences">
          ×
        </button>
      </header>

      <div className="preferences-tabs" role="tablist">
        <button
          type="button"
          className={`preferences-tab ${activeTab === 'audio' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('audio')}
          role="tab"
          aria-selected={activeTab === 'audio'}
        >
          Audio
        </button>
        <button
          type="button"
          className={`preferences-tab ${activeTab === 'recording' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('recording')}
          role="tab"
          aria-selected={activeTab === 'recording'}
        >
          Recording
        </button>
        <button
          type="button"
          className={`preferences-tab ${activeTab === 'osc' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('osc')}
          role="tab"
          aria-selected={activeTab === 'osc'}
        >
          OSC
        </button>
        <button
          type="button"
          className={`preferences-tab ${activeTab === 'suno' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('suno')}
          role="tab"
          aria-selected={activeTab === 'suno'}
        >
          Suno
        </button>
      </div>

      <section className="preferences-content">
        {activeTab === 'audio' ? (
          <div className="preferences-panel" role="tabpanel">
            <div className="preference-item">
              <label>
                <span className="label-text">Audio Device</span>
                <select value={tempAudioConfig.deviceId ?? ''} onChange={handleDeviceChange}>
                  <option value="">System Default</option>
                  {audioDevices.map((device) => (
                    <option key={device.name} value={device.name}>
                      {device.name} ({device.maxOutputChannels} ch)
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <div className="preference-item">
              <span className="label-text">Output Routing</span>
            </div>

            <div className="channel-grid">
              {renderChannelSelector('main', 'Main Output')}
              {renderChannelSelector('cue', 'Cue Output')}
            </div>
          </div>
        ) : activeTab === 'recording' ? (
          <div className="preferences-panel" role="tabpanel">
            {tempRecordingConfig && (
              <>
                <div className="preference-item">
                  <label>
                    <span className="label-text">Recording Directory</span>
                    <input
                      type="text"
                      value={tempRecordingConfig.directory}
                      onChange={handleRecordingDirectoryChange}
                      placeholder="/Users/you/Music/Sujay Recordings"
                      spellCheck={false}
                    />
                  </label>
                </div>

                <div className="preference-item">
                  <label>
                    <span className="label-text">Recording Format</span>
                    <select value={tempRecordingConfig.format} onChange={handleRecordingFormatChange}>
                      <option value="wav">WAV</option>
                      <option value="ogg">OGG (Vorbis)</option>
                    </select>
                  </label>
                </div>
              </>
            )}
          </div>
        ) : activeTab === 'osc' ? (
          <div className="preferences-panel" role="tabpanel">
            <div className="preference-item">
              <label className="checkbox-label">
                <input type="checkbox" checked={tempConfig.enabled} onChange={handleEnabledChange} />
                <span>Enable OSC Broadcasting</span>
              </label>
            </div>

            <div className="preference-item">
              <label>
                <span className="label-text">Host</span>
                <input
                  type="text"
                  value={tempConfig.host}
                  onChange={handleHostChange}
                  disabled={!tempConfig.enabled}
                  placeholder="127.0.0.1"
                />
              </label>
            </div>

            <div className="preference-item">
              <label>
                <span className="label-text">Port</span>
                <input
                  type="number"
                  value={tempConfig.port}
                  onChange={handlePortChange}
                  disabled={!tempConfig.enabled}
                  min="1"
                  max="65535"
                />
              </label>
            </div>
          </div>
        ) : (
          <div className="preferences-panel" role="tabpanel">
            <div className="preference-item">
              <label>
                <span className="label-text">Suno Session Cookie</span>
                <textarea
                  value={tempSunoConfig.cookie}
                  onChange={handleSunoCookieChange}
                  placeholder="__client=...; __session=...;"
                  spellCheck={false}
                />
              </label>
            </div>
          </div>
        )}
      </section>

      <footer className="preferences-footer">
        <button className="button-secondary" onClick={handleCancel} type="button">
          Cancel
        </button>
        <button className="button-primary" onClick={handleSave} type="button">
          Save
        </button>
      </footer>
    </div>
  );
};

export default Preferences;
