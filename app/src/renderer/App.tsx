import React, { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import type { AudioEngineState, RecordingStatus } from '../types';
import Notification from './components/Notification';
import '../assets/fonts/PixelMplus12-Regular.ttf';
import './App.css';

type NativeUIFrame = {
  x: number;
  y: number;
  width: number;
  height: number;
};

const App: React.FC = () => {
  const [audioState, setAudioState] = useState<AudioEngineState>({
    currentTrack: null,
    nextTrack: null,
    position: 0,
    nextPosition: 0,
    isPlaying: false,
    isCrossfading: false,
    crossfadeProgress: 0,
    deckA: null,
    deckB: null,
    deckAPosition: 0,
    deckBPosition: 0,
    deckAPlaying: false,
    deckBPlaying: false,
    crossfaderPosition: 0,
    masterTempo: 130,
    deckAPeak: 0,
    deckBPeak: 0,
    deckAPeakHold: 0,
    deckBPeakHold: 0,
    deckACueEnabled: false,
    deckBCueEnabled: false,
    micAvailable: false,
    micEnabled: false,
    micWarning: null,
    talkoverActive: false,
    talkoverButtonPressed: false,
    micLevel: 0,
  });

  const nativeUIAnchorRef = useRef<HTMLDivElement>(null);
  const micLevelFillRef = useRef<HTMLDivElement>(null);
  const [nativeUiReady, setNativeUiReady] = useState<boolean>(false);
  const [notification, setNotification] = useState<string | null>(null);
  const [systemInfo, setSystemInfo] = useState<{ time: string; cpuUsage: number; memoryUsage: number }>({ time: '--:--:--', cpuUsage: 0, memoryUsage: 0 });
  const [recordingStatus, setRecordingStatus] = useState<RecordingStatus>({ state: 'idle' });
  const recordingStatusRef = useRef(recordingStatus);

  useEffect(() => {
    recordingStatusRef.current = recordingStatus;
  }, [recordingStatus]);

  useEffect(() => {
    let mounted = true;

    const refreshAudioState = async () => {
      const nextAudio = await window.electronAPI.audioGetState();
      if (!mounted) {
        return;
      }
      setAudioState(nextAudio);
      if (micLevelFillRef.current) {
        const micLevel = Math.max(0, Math.min(1, nextAudio.micLevel ?? 0));
        micLevelFillRef.current.style.width = `${micLevel * 100}%`;
      }
    };

    const initializeStates = async () => {
      const recording = await window.electronAPI.recordingGetStatus();

      if (mounted) {
        await refreshAudioState();
        setRecordingStatus(recording);
        recordingStatusRef.current = recording;
      }
    };

    initializeStates();

    let notificationTimer: NodeJS.Timeout | null = null;
    const handleNotification = (message: string) => {
      if (mounted) {
        setNotification(message);
        if (notificationTimer) {
          clearTimeout(notificationTimer);
        }
        notificationTimer = setTimeout(() => {
          if (mounted) {
            setNotification(null);
          }
        }, 3000);
      }
    };

    const handleRecordingStatus = (status: RecordingStatus) => {
      if (!mounted) {
        return;
      }
      recordingStatusRef.current = status;
      setRecordingStatus(status);
    };

    const unsubscribeNotification = window.electronAPI.onNotification(handleNotification);
    const unsubscribeRecordingStatus = window.electronAPI.onRecordingStatus(handleRecordingStatus);

    const audioStatePollingTimer = setInterval(() => {
      void refreshAudioState();
    }, 500);

    return () => {
      mounted = false;
      clearInterval(audioStatePollingTimer);
      if (notificationTimer) {
        clearTimeout(notificationTimer);
      }
      unsubscribeNotification();
      unsubscribeRecordingStatus();
    };
  }, []);

  // System info polling
  useEffect(() => {
    let mounted = true;

    const updateSystemInfo = async () => {
      if (!mounted) return;
      try {
        const info = await window.electronAPI.getSystemInfo();
        if (mounted) {
          setSystemInfo(info);
        }
      } catch (error) {
        console.error('Error fetching system info:', error);
      }
    };

    updateSystemInfo();
    const interval = setInterval(updateSystemInfo, 1000);

    return () => {
      mounted = false;
      clearInterval(interval);
    };
  }, []);

  const handleMicEnabledChange = useCallback(async (enabled: boolean) => {
    await window.electronAPI.audioSetMicEnabled(enabled);
    setAudioState((prev) => ({ ...prev, micEnabled: enabled }));
  }, []);

  const handleRecordingAction = useCallback(async (action: 'start' | 'stop') => {
    try {
      const nextStatus = action === 'start'
        ? (async () => {
            const config = await window.electronAPI.recordingGetConfig();
            return await window.electronAPI.recordingStart(config.format);
          })()
        : await window.electronAPI.recordingStop();
      recordingStatusRef.current = nextStatus;
      setRecordingStatus(nextStatus);
    } catch (error) {
      console.error('Recording operation failed:', error);
      const message = error instanceof Error ? error.message : 'Recording operation failed';
      setNotification(`Recording error: ${message}`);
    }
  }, [setNotification]);

  const currentTrackArtwork = useMemo(() => {
    return audioState.deckA?.cachedImageData;
  }, [audioState.deckA?.cachedImageData]);

  const nextTrackArtwork = useMemo(() => {
    return audioState.deckB?.cachedImageData;
  }, [audioState.deckB?.cachedImageData]);

  const micAvailable = audioState.micAvailable ?? false;
  const micEnabled = audioState.micEnabled ?? false;

  // MIC ボタンのスタイル
  const micPillClass = !micAvailable ? 'is-unavailable' : micEnabled ? 'is-on' : 'is-off';

  const recordingState = recordingStatus.state;
  const recordingActive = recordingState === 'recording';
  const recordingBusy = recordingState === 'preparing' || recordingState === 'stopping';
  
  const [recordingElapsed, setRecordingElapsed] = useState(0);
  
  useEffect(() => {
    if (!recordingActive || !recordingStatus.activeFile) {
      setRecordingElapsed(0);
      return;
    }
    
    const startTime = recordingStatus.activeFile.createdAt;
    const updateElapsed = () => {
      const elapsed = Date.now() - startTime;
      setRecordingElapsed(elapsed);
    };
    
    updateElapsed();
    const timer = setInterval(updateElapsed, 100);
    
    return () => clearInterval(timer);
  }, [recordingActive, recordingStatus.activeFile]);
  
  const recordingButtonLabel = useMemo(() => {
    if (recordingActive && recordingStatus.activeFile) {
      const seconds = Math.floor(recordingElapsed / 1000);
      const minutes = Math.floor(seconds / 60);
      const secs = seconds % 60;
      return `${minutes}:${secs.toString().padStart(2, '0')}`;
    }
    return 'REC';
  }, [recordingActive, recordingStatus.activeFile, recordingElapsed]);
  
  const recordingStatusLabel = useMemo(() => {
    if (recordingStatus.lastError) {
      return `Error: ${recordingStatus.lastError}`;
    }
    if (recordingState === 'preparing') {
      return 'Preparing…';
    }
    if (recordingState === 'stopping') {
      return 'Stopping…';
    }
    return '';
  }, [recordingStatus, recordingState]);

  const handleRecordingButtonClick = useCallback(() => {
    if (recordingBusy) {
      return;
    }
    const action = recordingActive ? 'stop' : 'start';
    handleRecordingAction(action);
  }, [recordingBusy, recordingActive, handleRecordingAction]);

  useEffect(() => {
    let attached = false;
    let rafId: number | null = null;
    let lastSentFrame: NativeUIFrame | null = null;
    const anchor = nativeUIAnchorRef.current;
    if (!anchor) {
      return;
    }

    const sendFrame = async (isInitial: boolean) => {
      const rect = anchor.getBoundingClientRect();
      const frame: NativeUIFrame = {
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      };

      if (frame.width <= 0 || frame.height <= 0) {
        return;
      }

      const isSameFrame =
        lastSentFrame !== null &&
        lastSentFrame.x === frame.x &&
        lastSentFrame.y === frame.y &&
        lastSentFrame.width === frame.width &&
        lastSentFrame.height === frame.height;

      if (!isInitial && isSameFrame) {
        return;
      }

      try {
        if (isInitial) {
          attached = await window.electronAPI.nativeUiAttach(frame);
        } else {
          attached = await window.electronAPI.nativeUiSetFrame(frame);
        }
        lastSentFrame = frame;
        setNativeUiReady(attached);
      } catch (error) {
        console.error('[native-ui] failed to sync frame:', error);
        setNativeUiReady(false);
      }
    };

    const scheduleFrameSync = (isInitial = false) => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
      }
      rafId = requestAnimationFrame(() => {
        rafId = null;
        void sendFrame(isInitial || !attached);
      });
    };

    const observer = new ResizeObserver(() => {
      scheduleFrameSync();
    });
    observer.observe(anchor);

    const onWindowResized = () => {
      scheduleFrameSync();
    };

    window.addEventListener('resize', onWindowResized);
    scheduleFrameSync(true);

    return () => {
      observer.disconnect();
      window.removeEventListener('resize', onWindowResized);
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
      }
      if (attached) {
        void window.electronAPI.nativeUiDetach();
      }
      setNativeUiReady(false);
    };
  }, []);

  // Send artwork to native UI when tracks change
  const prevArtworkA = useRef<string | undefined>(undefined);
  const prevArtworkB = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!nativeUiReady) return;

    const sendArtwork = (deck: 1 | 2, dataUrl: string | undefined, prevRef: React.MutableRefObject<string | undefined>) => {
      if (dataUrl === prevRef.current) return;
      prevRef.current = dataUrl;
      if (!dataUrl) {
        window.electronAPI.nativeUiClearArtwork(deck);
        return;
      }
      const img = new Image();
      img.onload = () => {
        const canvas = document.createElement('canvas');
        const size = 64; // small thumbnail
        canvas.width = size;
        canvas.height = size;
        const ctx2d = canvas.getContext('2d');
        if (!ctx2d) return;
        ctx2d.drawImage(img, 0, 0, size, size);
        const imageData = ctx2d.getImageData(0, 0, size, size);
        window.electronAPI.nativeUiSetArtwork(deck, size, size, imageData.data);
      };
      img.src = dataUrl;
    };

    sendArtwork(1, currentTrackArtwork, prevArtworkA);
    sendArtwork(2, nextTrackArtwork, prevArtworkB);
  }, [nativeUiReady, currentTrackArtwork, nextTrackArtwork]);

  return (
    <div className="app">
      <div className="titlebar-overlay">
        <div className="titlebar-title">{document.title}</div>
        <div className="titlebar-info">
          <div className="titlebar-recording" title={recordingStatus.activeFile?.path || recordingStatusLabel}>
            <button
              type="button"
              className={`recording-pill ${recordingActive ? 'is-active' : ''} ${recordingStatus.lastError ? 'has-error' : ''}`}
              onClick={handleRecordingButtonClick}
              disabled={recordingBusy}
              aria-pressed={recordingActive}
            >
              <span
                className={`recording-indicator ${recordingActive ? 'is-on' : ''} ${recordingStatus.lastError ? 'is-error' : ''}`}
                aria-hidden="true"
              />
              {recordingButtonLabel}
            </button>
            {recordingStatusLabel && <div className="recording-status-text">{recordingStatusLabel}</div>}
          </div>
          <div className="titlebar-mic">
            <button 
              className={`mic-pill ${micPillClass}`}
              onClick={() => handleMicEnabledChange(!micEnabled)}
              disabled={!micAvailable}
            >
              <span
                className={`mic-indicator ${micEnabled ? 'is-on' : ''}`}
                aria-hidden="true"
              />
              MIC
            </button>
            <div className="mic-level-bar">
              <div className="mic-level-fill" ref={micLevelFillRef} />
            </div>
          </div>
          <span className="cpu-label">CPU</span>
          <div className="cpu-bar">
            <div 
              className="cpu-bar-fill" 
              style={{ width: `${Math.min(100, systemInfo.cpuUsage)}%` }}
            ></div>
          </div>
          <span className="cpu-value">{systemInfo.cpuUsage.toFixed(1)}%</span>
          <div className="titlebar-separator"></div>
          <span className="mem-label">MEM</span>
          <span className="mem-value">{systemInfo.memoryUsage}MB</span>
          <div className="titlebar-separator"></div>
          <span className="time">{systemInfo.time}</span>
        </div>
      </div>
      <div className="console-shell">
        <div className="native-ui-anchor" ref={nativeUIAnchorRef} aria-hidden="true" />
      </div>

      {notification && <Notification message={notification} />}
    </div>
  );
};

export default App;
