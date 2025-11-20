import React, { useState, useEffect } from 'react';
import type { OSCConfig } from '../../types';
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

  const [tempConfig, setTempConfig] = useState<OSCConfig>(oscConfig);

  useEffect(() => {
    if (isOpen) {
      // Load current OSC config
      window.electronAPI.oscGetConfig().then((config) => {
        setOscConfig(config);
        setTempConfig(config);
      });
    }
  }, [isOpen]);

  const handleSave = async () => {
    await window.electronAPI.oscUpdateConfig(tempConfig);
    setOscConfig(tempConfig);
    onClose();
  };

  const handleCancel = () => {
    setTempConfig(oscConfig);
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
