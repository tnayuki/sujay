import React, { useState, useEffect } from 'react';
import type { OSCConfig, AudioConfig, AudioDevice } from '../../types';
import './Preferences.css';

type PreferencesTab = 'audio' | 'osc';

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
  const [selectedDevice, setSelectedDevice] = useState<AudioDevice | null>(null);

  const [tempConfig, setTempConfig] = useState<OSCConfig>(oscConfig);
  const [tempAudioConfig, setTempAudioConfig] = useState<AudioConfig>(audioConfig);

  useEffect(() => {
    let mounted = true;

    const loadPreferences = async () => {
      try {
        const [osc, devices, audio] = await Promise.all([
          window.electronAPI.oscGetConfig(),
          window.electronAPI.audioGetDevices(),
          window.electronAPI.audioGetConfig(),
        ]);

        if (!mounted) {
          return;
        }

        setOscConfig(osc);
        setTempConfig(osc);
        setAudioDevices(devices);
        setAudioConfig(audio);
        setTempAudioConfig(audio);
        setSelectedDevice(devices.find((d) => d.id === audio.deviceId) || null);
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
      await Promise.all([
        window.electronAPI.oscUpdateConfig(tempConfig),
        window.electronAPI.audioUpdateConfig(tempAudioConfig),
      ]);
      setOscConfig(tempConfig);
      setAudioConfig(tempAudioConfig);
      onClose();
    } catch (error) {
      console.error('Failed to save preferences', error);
    }
  };

  const handleCancel = () => {
    setTempConfig(oscConfig);
    setTempAudioConfig(audioConfig);
    onClose();
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

  const handleDeviceChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const deviceId = parseInt(e.target.value, 10);
    const device = audioDevices.find((d) => d.id === deviceId) || null;
    setSelectedDevice(device);

    const channelLimit = device ? device.maxOutputChannels : 2;
    const clampPair = (pair: [number | null, number | null]): [number | null, number | null] => [
      pair[0] !== null && pair[0] >= channelLimit ? null : pair[0],
      pair[1] !== null && pair[1] >= channelLimit ? null : pair[1],
    ];

    setTempAudioConfig({
      ...tempAudioConfig,
      deviceId: deviceId >= 0 ? deviceId : undefined,
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

  const channelOptions = selectedDevice ? Array.from({ length: selectedDevice.maxOutputChannels }, (_, i) => i) : [];

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
            disabled={!selectedDevice}
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
            disabled={!selectedDevice}
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
          className={`preferences-tab ${activeTab === 'osc' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('osc')}
          role="tab"
          aria-selected={activeTab === 'osc'}
        >
          OSC
        </button>
      </div>

      <section className="preferences-content">
        {activeTab === 'audio' ? (
          <div className="preferences-panel" role="tabpanel">
            <div className="preference-item">
              <label>
                <span className="label-text">Audio Device</span>
                <select value={tempAudioConfig.deviceId ?? -1} onChange={handleDeviceChange}>
                  <option value={-1}>System Default</option>
                  {audioDevices.map((device) => (
                    <option key={device.id} value={device.id}>
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
        ) : (
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
