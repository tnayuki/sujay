import React, { useState, useEffect } from 'react';
import type { OSCConfig, AudioConfig, AudioDevice } from '../../types';
import './Preferences.css';

interface PreferencesProps {
  isOpen: boolean;
  onClose: () => void;
}

const Preferences: React.FC<PreferencesProps> = ({ isOpen, onClose }) => {
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
    if (isOpen) {
      // Load current OSC config
      window.electronAPI.oscGetConfig().then((config) => {
        setOscConfig(config);
        setTempConfig(config);
      });
      
      // Load audio devices and current audio config
      Promise.all([
        window.electronAPI.audioGetDevices(),
        window.electronAPI.audioGetConfig()
      ]).then(([devices, config]) => {
        setAudioDevices(devices);
        setAudioConfig(config);
        setTempAudioConfig(config);
        
        const device = devices.find((d: AudioDevice) => d.id === config.deviceId);
        setSelectedDevice(device || null);
      });
    }
  }, [isOpen]);

  const handleSave = async () => {
    await Promise.all([
      window.electronAPI.oscUpdateConfig(tempConfig),
      window.electronAPI.audioUpdateConfig(tempAudioConfig)
    ]);
    setOscConfig(tempConfig);
    setAudioConfig(tempAudioConfig);
    onClose();
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
    const device = audioDevices.find(d => d.id === deviceId);
    setSelectedDevice(device || null);
    
    // Adjust channels out of range to null if device changes
    const cap = device ? device.maxOutputChannels : 2;
    const clampPair = (pair: [number | null, number | null]): [number | null, number | null] => [
      pair[0] !== null && pair[0] >= cap ? null : pair[0],
      pair[1] !== null && pair[1] >= cap ? null : pair[1],
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

  if (!isOpen) return null;

  return (
    <div className="preferences-overlay" onClick={handleCancel}>
      <div className="preferences-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="preferences-header">
          <h2>Preferences</h2>
          <button className="close-button" onClick={handleCancel}>
            ×
          </button>
        </div>

        <div className="preferences-content">
          <section className="preferences-section">
            <h3>Audio Settings</h3>
            
            <div className="preference-item">
              <label>
                <span className="label-text">Audio Device:</span>
                <select 
                  value={tempAudioConfig.deviceId ?? -1} 
                  onChange={handleDeviceChange}
                >
                  <option value={-1}>Default Device</option>
                  {audioDevices.map(device => (
                    <option key={device.id} value={device.id}>
                      {device.name} ({device.maxOutputChannels} channels)
                    </option>
                  ))}
                </select>
              </label>
            </div>

            {selectedDevice && selectedDevice.maxOutputChannels >= 2 && (
              <div className="channel-selections">
                <div className="preference-item channel-card">
                  <label>
                    <span className="label-text">Main Output Channels:</span>
                    <div className="channel-controls">
                      <div className="channel-pair">
                        <span>L:</span>
                        <select 
                          value={tempAudioConfig.mainChannels[0] ?? -1} 
                          onChange={(e) => {
                            const v = parseInt(e.target.value, 10);
                            handleChannelChange('main', 'left', v === -1 ? null : v);
                          }}
                        >
                          <option value={-1}>-</option>
                          {Array.from({ length: selectedDevice.maxOutputChannels }, (_, i) => (
                            <option key={i} value={i}>{i + 1}</option>
                          ))}
                        </select>
                        <span>R:</span>
                        <select 
                          value={tempAudioConfig.mainChannels[1] ?? -1} 
                          onChange={(e) => {
                            const v = parseInt(e.target.value, 10);
                            handleChannelChange('main', 'right', v === -1 ? null : v);
                          }}
                        >
                          <option value={-1}>-</option>
                          {Array.from({ length: selectedDevice.maxOutputChannels }, (_, i) => (
                            <option key={i} value={i}>{i + 1}</option>
                          ))}
                        </select>
                      </div>
                    </div>
                  </label>
                </div>

                <div className="preference-item channel-card">
                  <label>
                    <span className="label-text">Cue Output Channels:</span>
                    <div className="channel-controls">
                      <div className="channel-pair">
                        <span>L:</span>
                        <select 
                          value={tempAudioConfig.cueChannels[0] ?? -1} 
                          onChange={(e) => {
                            const v = parseInt(e.target.value, 10);
                            handleChannelChange('cue', 'left', v === -1 ? null : v);
                          }}
                        >
                          <option value={-1}>-</option>
                          {Array.from({ length: selectedDevice.maxOutputChannels }, (_, i) => (
                            <option key={i} value={i}>{i + 1}</option>
                          ))}
                        </select>
                        <span>R:</span>
                        <select 
                          value={tempAudioConfig.cueChannels[1] ?? -1} 
                          onChange={(e) => {
                            const v = parseInt(e.target.value, 10);
                            handleChannelChange('cue', 'right', v === -1 ? null : v);
                          }}
                        >
                          <option value={-1}>-</option>
                          {Array.from({ length: selectedDevice.maxOutputChannels }, (_, i) => (
                            <option key={i} value={i}>{i + 1}</option>
                          ))}
                        </select>
                      </div>
                    </div>
                  </label>
                </div>
              </div>
            )}
          </section>

          <section className="preferences-section">
            <h3>OSC Settings</h3>
            
            <div className="preference-item">
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={tempConfig.enabled}
                  onChange={handleEnabledChange}
                />
                <span>Enable OSC</span>
              </label>
            </div>

            <div className="preference-item">
              <label>
                <span className="label-text">Host:</span>
                <input
                  type="text"
                  value={tempConfig.host}
                  onChange={handleHostChange}
                  disabled={!tempConfig.enabled}
                  placeholder="127.0.0.1"
                />
              </label>
              <span className="help-text">
                IP address to send OSC messages (e.g., 127.0.0.1 for localhost, 255.255.255.255 for broadcast)
              </span>
            </div>

            <div className="preference-item">
              <label>
                <span className="label-text">Port:</span>
                <input
                  type="number"
                  value={tempConfig.port}
                  onChange={handlePortChange}
                  disabled={!tempConfig.enabled}
                  min="1"
                  max="65535"
                  placeholder="9000"
                />
              </label>
              <span className="help-text">
                Port number to send OSC messages (1-65535)
              </span>
            </div>
          </section>
        </div>

        <div className="preferences-footer">
          <button className="button-secondary" onClick={handleCancel}>
            Cancel
          </button>
          <button className="button-primary" onClick={handleSave}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
};

export default Preferences;
