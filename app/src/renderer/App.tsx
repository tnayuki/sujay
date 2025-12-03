import React, { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import type { AudioEngineState, AudioLevelState, LibraryState, RecordingStatus, Track, Workspace, EqBand, TrackStructure } from '../types';
import type { AudioInfo } from '../suno-api';
import Console from './components/Console';
import Library from './components/Library';
import Notification from './components/Notification';
import '../assets/fonts/PixelMplus12-Regular.ttf';
import '../assets/fonts/DSEG7Classic-Regular.ttf';
import './App.css';

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

  type WaveformBuffer = {
    chunks: (number[] | null)[];
    totalChunks: number;
  };

  const deckAWaveformRef = useRef<(number[] | Float32Array) | null>(null);
  const deckBWaveformRef = useRef<(number[] | Float32Array) | null>(null);
  const deckAStructureRef = useRef<{ trackId: string; structure: TrackStructure } | null>(null);
  const deckBStructureRef = useRef<{ trackId: string; structure: TrackStructure } | null>(null);
  const waveformBuffersRef = useRef<Record<string, WaveformBuffer>>({});
  const [waveformVersion, forceWaveformRender] = useState<number>(0);
  const audioStateRef = useRef<AudioEngineState>(audioState);

  useEffect(() => {
    audioStateRef.current = audioState;
  }, [audioState]);

  const [libraryState, setLibraryState] = useState<LibraryState>({
    tracks: [],
    workspaces: [],
    selectedWorkspace: null,
    likedFilter: false,
    syncing: false,
  });

  const [syncStatus, setSyncStatus] = useState<{
    syncing: boolean;
    progress?: { current: number; message: string };
  }>({ syncing: false });

  const [downloadProgress, setDownloadProgress] = useState<Map<string, string>>(new Map());
  const [notification, setNotification] = useState<string | null>(null);
  const [systemInfo, setSystemInfo] = useState<{ time: string; cpuUsage: number }>({ time: '--:--:--', cpuUsage: 0 });
  const isLoadingTrackRef = useRef(false);
  const [recordingStatus, setRecordingStatus] = useState<RecordingStatus>({ state: 'idle' });
  const recordingStatusRef = useRef(recordingStatus);

  useEffect(() => {
    recordingStatusRef.current = recordingStatus;
  }, [recordingStatus]);

  useEffect(() => {
    let mounted = true;

    const initializeStates = async () => {
      const audio = await window.electronAPI.audioGetState();
      const library = await window.electronAPI.libraryGetState();
      const progress = await window.electronAPI.libraryGetDownloadProgress();
      const recording = await window.electronAPI.recordingGetStatus();

      if (mounted) {
        setAudioState(audio);
        setLibraryState(library);
        setDownloadProgress(new Map(progress));
        setRecordingStatus(recording);
        recordingStatusRef.current = recording;
      }
    };

    initializeStates();

    const stripWaveformData = (track: Track | null | undefined): Track | null => {
      if (!track) {
        return null;
      }
      return { ...track, waveformData: undefined as Track['waveformData'] };
    };

    const handleAudioStateChanged = (state: AudioEngineState) => {
      if (!mounted) return;

      const prevDeckAId = audioStateRef.current.deckA?.id;
      const prevDeckBId = audioStateRef.current.deckB?.id;
      const newDeckAId = state.deckA?.id;
      const newDeckBId = state.deckB?.id;

      // Clean up waveform data when track changes
      if (state.deckA && prevDeckAId !== newDeckAId) {
        deckAWaveformRef.current = null;
        forceWaveformRender((v: number) => v + 1);
        if (prevDeckAId && prevDeckAId !== newDeckBId) {
          delete waveformBuffersRef.current[prevDeckAId];
        }
      }
      if (state.deckB && prevDeckBId !== newDeckBId) {
        deckBWaveformRef.current = null;
        forceWaveformRender((v: number) => v + 1);
        if (prevDeckBId && prevDeckBId !== newDeckAId) {
          delete waveformBuffersRef.current[prevDeckBId];
        }
      }

      // Merge track info only when included (track changed)
      const cleanedState: AudioEngineState = {
        ...state,
        deckA: state.deckA !== undefined ? stripWaveformData(state.deckA) : audioStateRef.current.deckA,
        deckB: state.deckB !== undefined ? stripWaveformData(state.deckB) : audioStateRef.current.deckB,
        deckAPosition: state.deckAPosition !== undefined ? state.deckAPosition : audioStateRef.current.deckAPosition,
        deckBPosition: state.deckBPosition !== undefined ? state.deckBPosition : audioStateRef.current.deckBPosition,
        deckAPlaying: state.deckAPlaying !== undefined ? state.deckAPlaying : audioStateRef.current.deckAPlaying,
        deckBPlaying: state.deckBPlaying !== undefined ? state.deckBPlaying : audioStateRef.current.deckBPlaying,
        isPlaying: state.isPlaying !== undefined ? state.isPlaying : audioStateRef.current.isPlaying,
        isCrossfading: state.isCrossfading !== undefined ? state.isCrossfading : audioStateRef.current.isCrossfading,
        crossfaderPosition: state.crossfaderPosition !== undefined ? state.crossfaderPosition : (audioStateRef.current.crossfaderPosition ?? 0),
        masterTempo: state.masterTempo !== undefined ? state.masterTempo : (audioStateRef.current.masterTempo ?? 130),
        deckAGain: state.deckAGain !== undefined ? state.deckAGain : (audioStateRef.current.deckAGain ?? 1.0),
        deckBGain: state.deckBGain !== undefined ? state.deckBGain : (audioStateRef.current.deckBGain ?? 1.0),
        deckACueEnabled: state.deckACueEnabled !== undefined ? state.deckACueEnabled : (audioStateRef.current.deckACueEnabled ?? false),
        deckBCueEnabled: state.deckBCueEnabled !== undefined ? state.deckBCueEnabled : (audioStateRef.current.deckBCueEnabled ?? false),
        deckAPeak: state.deckAPeak !== undefined ? state.deckAPeak : audioStateRef.current.deckAPeak,
        deckBPeak: state.deckBPeak !== undefined ? state.deckBPeak : audioStateRef.current.deckBPeak,
        deckAPeakHold: state.deckAPeakHold !== undefined ? state.deckAPeakHold : audioStateRef.current.deckAPeakHold,
        deckBPeakHold: state.deckBPeakHold !== undefined ? state.deckBPeakHold : audioStateRef.current.deckBPeakHold,
        micAvailable: state.micAvailable !== undefined ? state.micAvailable : audioStateRef.current.micAvailable,
        micEnabled: state.micEnabled !== undefined ? state.micEnabled : audioStateRef.current.micEnabled,
        talkoverActive: state.talkoverActive !== undefined ? state.talkoverActive : audioStateRef.current.talkoverActive,
        talkoverButtonPressed: state.talkoverButtonPressed !== undefined ? state.talkoverButtonPressed : audioStateRef.current.talkoverButtonPressed,
        micLevel: state.micLevel !== undefined ? state.micLevel : audioStateRef.current.micLevel,
        micWarning: state.micWarning !== undefined ? state.micWarning : audioStateRef.current.micWarning,
        currentTrack: state.currentTrack !== undefined ? stripWaveformData(state.currentTrack || null) || undefined : audioStateRef.current.currentTrack,
        nextTrack: state.nextTrack !== undefined ? stripWaveformData(state.nextTrack || null) || undefined : audioStateRef.current.nextTrack,
        position: state.position !== undefined ? state.position : audioStateRef.current.position,
        nextPosition: state.nextPosition !== undefined ? state.nextPosition : audioStateRef.current.nextPosition,
      };

      audioStateRef.current = cleanedState;
      setAudioState(cleanedState);
    };

    const handleAudioLevelState = (levelState: AudioLevelState) => {
      if (!mounted) return;
      
      // Update only level fields for meters
      const updatedState: AudioEngineState = {
        ...audioStateRef.current,
        deckAPeak: levelState.deckAPeak,
        deckBPeak: levelState.deckBPeak,
        deckAPeakHold: levelState.deckAPeakHold,
        deckBPeakHold: levelState.deckBPeakHold,
        micLevel: levelState.micLevel,
        talkoverActive: levelState.talkoverActive,
        talkoverButtonPressed: levelState.talkoverButtonPressed ?? audioStateRef.current.talkoverButtonPressed,
      };
      
      audioStateRef.current = updatedState;
      setAudioState(updatedState);
    };

    const handleLibraryStateChanged = (state: LibraryState) => {
      if (mounted) {
        setLibraryState(state);
      }
    };

    let downloadProgressTimeout: NodeJS.Timeout | null = null;
    const handleDownloadProgressChanged = (progress: Map<string, string>) => {
      if (!mounted) return;

      if (downloadProgressTimeout) {
        clearTimeout(downloadProgressTimeout);
      }

      downloadProgressTimeout = setTimeout(() => {
        if (mounted) {
          setDownloadProgress(new Map(progress));
        }
      }, 500);
    };

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

    const handleLibrarySyncStarted = () => setSyncStatus({ syncing: true });
    const handleLibrarySyncProgress = (data: { current: number; total: number; message?: string }) => {
      if (mounted) {
        setSyncStatus({
          syncing: true,
          progress: {
            current: data.current,
            message: data.message,
          },
        });
      }
    };
    const handleLibrarySyncCompleted = () => setSyncStatus({ syncing: false });
    const handleLibrarySyncFailed = () => setSyncStatus({ syncing: false });

    const handleWaveformChunk = (data: { trackId: string; chunkIndex: number; totalChunks: number; chunk: number[] }) => {
      if (!mounted) return;

      const { trackId, chunkIndex, totalChunks, chunk } = data;
      const buffers = waveformBuffersRef.current;
      const buffer = buffers[trackId] || {
        chunks: Array(totalChunks).fill(null),
        totalChunks,
      };

      buffer.chunks[chunkIndex] = chunk;
      buffers[trackId] = buffer;
    };

    const handleWaveformComplete = ({ trackId }: { trackId: string; totalFrames: number }) => {
      if (!mounted) return;

      const buffer = waveformBuffersRef.current[trackId];
      if (!buffer) {
        return;
      }

      // Directly concatenate chunks into a single array
      const combinedWaveform: number[] = [];
      for (const chunk of buffer.chunks) {
        if (chunk) {
          combinedWaveform.push(...chunk);
        }
      }

      const latestState = audioStateRef.current;
      if (latestState.deckA?.id === trackId) {
        deckAWaveformRef.current = combinedWaveform;
        forceWaveformRender((v: number) => v + 1);
      } else if (latestState.deckB?.id === trackId) {
        deckBWaveformRef.current = combinedWaveform;
        forceWaveformRender((v: number) => v + 1);
      }

      delete waveformBuffersRef.current[trackId];
    };

    const handleRecordingStatus = (status: RecordingStatus) => {
      if (!mounted) {
        return;
      }
      recordingStatusRef.current = status;
      setRecordingStatus(status);
    };

    const handleTrackStructure = ({ trackId, deck, structure }: { trackId: string; deck: 1 | 2; structure: TrackStructure }) => {
      if (!mounted) return;

      if (deck === 1) {
        deckAStructureRef.current = { trackId, structure };
      } else {
        deckBStructureRef.current = { trackId, structure };
      }
      // Force re-render so WaveformZoom gets the updated beats
      forceWaveformRender((v: number) => v + 1);
    };

    const unsubscribeAudio = window.electronAPI.onAudioStateChanged(handleAudioStateChanged);
    const unsubscribeLevel = window.electronAPI.onAudioLevelState(handleAudioLevelState);
    const unsubscribeLibrary = window.electronAPI.onLibraryStateChanged(handleLibraryStateChanged);
    const unsubscribeProgress = window.electronAPI.onDownloadProgressChanged(handleDownloadProgressChanged);
    const unsubscribeNotification = window.electronAPI.onNotification(handleNotification);
    const unsubscribeSyncStarted = window.electronAPI.onLibrarySyncStarted(handleLibrarySyncStarted);
    const unsubscribeSyncProgress = window.electronAPI.onLibrarySyncProgress(handleLibrarySyncProgress);
    const unsubscribeSyncCompleted = window.electronAPI.onLibrarySyncCompleted(handleLibrarySyncCompleted);
    const unsubscribeSyncFailed = window.electronAPI.onLibrarySyncFailed(handleLibrarySyncFailed);
    const unsubscribeTrackLoadDeck = window.electronAPI.onTrackLoadDeck(async (data) => {
      if (!mounted || isLoadingTrackRef.current) {
        return;
      }

      isLoadingTrackRef.current = true;

      try {
        const downloadedTrack = await window.electronAPI.libraryDownloadTrack(data.track);
        await window.electronAPI.audioLoadTrack(downloadedTrack, data.deck);
      } catch (error) {
        console.error('Error handling track load deck event:', error);
      } finally {
        isLoadingTrackRef.current = false;
      }
    });
    const unsubscribeWaveformChunk = window.electronAPI.onWaveformChunk(handleWaveformChunk);
    const unsubscribeWaveformComplete = window.electronAPI.onWaveformComplete(handleWaveformComplete);
    const unsubscribeRecordingStatus = window.electronAPI.onRecordingStatus(handleRecordingStatus);
    const unsubscribeTrackStructure = window.electronAPI.onTrackStructure(handleTrackStructure);

    return () => {
      mounted = false;
      if (notificationTimer) {
        clearTimeout(notificationTimer);
      }
      if (downloadProgressTimeout) {
        clearTimeout(downloadProgressTimeout);
      }
      unsubscribeAudio();
      unsubscribeLevel();
      unsubscribeLibrary();
      unsubscribeProgress();
      unsubscribeNotification();
      unsubscribeSyncStarted();
      unsubscribeSyncProgress();
      unsubscribeSyncCompleted();
      unsubscribeSyncFailed();
      unsubscribeTrackLoadDeck();
      unsubscribeWaveformChunk();
      unsubscribeWaveformComplete();
      unsubscribeRecordingStatus();
      unsubscribeTrackStructure();
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

  const handleTrackClick = useCallback(async (audioInfo: AudioInfo) => {
    if (isLoadingTrackRef.current) {
      console.log('Already loading a track, please wait...');
      return;
    }

    isLoadingTrackRef.current = true;

    try {
      const downloadedTrack = await window.electronAPI.libraryDownloadTrack(audioInfo);
      const currentState = await window.electronAPI.audioGetState();
      await window.electronAPI.audioPlay(downloadedTrack, currentState.isPlaying);
    } catch (error) {
      console.error('Error in handleTrackClick:', error);
    } finally {
      isLoadingTrackRef.current = false;
    }
  }, []);

  const handleTrackDownload = useCallback(async (audioInfo: AudioInfo) => {
    try {
      await window.electronAPI.libraryDownloadTrack(audioInfo);
    } catch (error) {
      console.error('Error downloading track:', error);
    }
  }, []);

  const handleStopClick = useCallback((deck: 1 | 2) => {
    window.electronAPI.audioStop(deck);
  }, []);

  const handlePlay = useCallback(async (deck: 1 | 2) => {
    await window.electronAPI.audioStartDeck(deck);
  }, []);

  const handleSeek = useCallback((deck: 1 | 2, position: number) => {
    window.electronAPI.audioSeek(deck, position);
  }, []);

  const handleCrossfaderChange = useCallback((position: number) => {
    window.electronAPI.audioSetCrossfader(position);
  }, []);

  const handleMasterTempoChange = useCallback((bpm: number) => {
    window.electronAPI.audioSetMasterTempo(bpm);
  }, []);

  const handleDeckCueToggle = useCallback((deck: 1 | 2, enabled: boolean) => {
    window.electronAPI.audioSetDeckCue(deck, enabled);
  }, []);

  const handleEqCutToggle = useCallback((deck: 1 | 2, band: EqBand, enabled: boolean) => {
    window.electronAPI.audioSetEqCut(deck, band, enabled);
  }, []);

  const handleDeckGainChange = useCallback((deck: 1 | 2, gain: number) => {
    window.electronAPI.audioSetDeckGain(deck, gain);
  }, []);

  const handleSetLoop = useCallback((deck: 1 | 2, beats: number) => {
    const track = deck === 1 ? audioState.deckA : audioState.deckB;
    const position = deck === 1 ? audioState.deckAPosition : audioState.deckBPosition;
    const masterTempo = audioState.masterTempo ?? 130;
    const structureRef = deck === 1 ? deckAStructureRef : deckBStructureRef;
    
    if (!track) {
      return;
    }
    
    // Get beat grid from structure ref
    const beatGrid = (structureRef.current?.trackId === track.id) 
      ? structureRef.current.structure.beats 
      : undefined;
    
    window.electronAPI.audioSetBeatLoop(deck, beats, masterTempo, position || 0, beatGrid);
  }, [audioState.deckA, audioState.deckB, audioState.deckAPosition, audioState.deckBPosition, audioState.masterTempo]);

  const handleClearLoop = useCallback((deck: 1 | 2) => {
    window.electronAPI.audioClearLoop(deck);
  }, []);

  const handleMicEnabledChange = useCallback((enabled: boolean) => {
    window.electronAPI.audioSetMicEnabled(enabled);
  }, []);

  const handleWorkspaceChange = useCallback((workspace: Workspace | null) => {
    window.electronAPI.librarySetWorkspace(workspace);
  }, []);

  const handleToggleLikedFilter = useCallback(() => {
    window.electronAPI.libraryToggleLikedFilter();
  }, []);

  const handleRecordingAction = useCallback(async (action: 'start' | 'stop') => {
    try {
      const nextStatus = action === 'start'
        ? await window.electronAPI.recordingStart()
        : await window.electronAPI.recordingStop();
      recordingStatusRef.current = nextStatus;
      setRecordingStatus(nextStatus);
    } catch (error) {
      console.error('Recording operation failed:', error);
      const message = error instanceof Error ? error.message : 'Recording operation failed';
      setNotification(`Recording error: ${message}`);
    }
  }, [setNotification]);

  const activeTrackIds = [audioState.deckA?.id, audioState.deckB?.id].filter((id): id is string => Boolean(id));

  const currentTrackWithWaveform = useMemo(() => {
    if (!audioState.deckA) return null;
    const libraryTrack = libraryState.tracks.find(t => t.id === audioState.deckA?.id) as (typeof libraryState.tracks[0] & { cachedImageData?: string }) | undefined;
    const cachedStructure = deckAStructureRef.current;
    return {
      ...audioState.deckA,
      waveformData: deckAWaveformRef.current || undefined,
      structure: (cachedStructure?.trackId === audioState.deckA.id) ? cachedStructure.structure : undefined,
      cachedImageData: libraryTrack?.cachedImageData,
    };
  }, [audioState.deckA, waveformVersion, libraryState.tracks]);

  const nextTrackWithWaveform = useMemo(() => {
    if (!audioState.deckB) return null;
    const libraryTrack = libraryState.tracks.find(t => t.id === audioState.deckB?.id) as (typeof libraryState.tracks[0] & { cachedImageData?: string }) | undefined;
    const cachedStructure = deckBStructureRef.current;
    return {
      ...audioState.deckB,
      waveformData: deckBWaveformRef.current || undefined,
      structure: (cachedStructure?.trackId === audioState.deckB.id) ? cachedStructure.structure : undefined,
      cachedImageData: libraryTrack?.cachedImageData,
    };
  }, [audioState.deckB, waveformVersion, libraryState.tracks]);

  const micAvailable = audioState.micAvailable ?? false;
  const micEnabled = audioState.micEnabled ?? false;
  const micLevelValue = Math.max(0, Math.min(1, audioState.micLevel ?? 0));

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
              <div className="mic-level-fill" style={{ width: `${micLevelValue * 100}%` }} />
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
          <span className="time">{systemInfo.time}</span>
        </div>
      </div>
      <Console
        currentTrack={currentTrackWithWaveform}
        nextTrack={nextTrackWithWaveform}
        position={audioState.deckAPosition || 0}
        nextPosition={audioState.deckBPosition || 0}
        isSeek={audioState.isSeek}
        deckAPlaying={audioState.deckAPlaying}
        deckBPlaying={audioState.deckBPlaying}
        deckAPeak={audioState.deckAPeak || 0}
        deckBPeak={audioState.deckBPeak || 0}
        deckAPeakHold={audioState.deckAPeakHold || 0}
        deckBPeakHold={audioState.deckBPeakHold || 0}
        deckACueEnabled={audioState.deckACueEnabled ?? false}
        deckBCueEnabled={audioState.deckBCueEnabled ?? false}
        deckAEqCut={audioState.deckAEqCut ?? { low: false, mid: false, high: false }}
        deckBEqCut={audioState.deckBEqCut ?? { low: false, mid: false, high: false }}
        deckAGain={audioState.deckAGain ?? 1.0}
        deckBGain={audioState.deckBGain ?? 1.0}
        deckALoop={audioState.deckALoop}
        deckBLoop={audioState.deckBLoop}
        isPlaying={audioState.isPlaying}
        isCrossfading={audioState.isCrossfading}
        crossfadeProgress={audioState.crossfadeProgress}
        crossfaderPosition={audioState.crossfaderPosition}
        masterTempo={audioState.masterTempo ?? 130}
        onStop={handleStopClick}
        onSeek={handleSeek}
        onCrossfaderChange={handleCrossfaderChange}
        onMasterTempoChange={handleMasterTempoChange}
        onDeckCueToggle={handleDeckCueToggle}
        onEqCutToggle={handleEqCutToggle}
        onDeckGainChange={handleDeckGainChange}
        onSetLoop={handleSetLoop}
        onClearLoop={handleClearLoop}
        onPlay={handlePlay}
      />

      <Library
        tracks={libraryState.tracks}
        workspaces={libraryState.workspaces}
        currentWorkspace={libraryState.selectedWorkspace}
        syncStatus={syncStatus}
        downloadProgress={downloadProgress}
        activeTrackIds={activeTrackIds}
        onTrackClick={handleTrackClick}
        onTrackDownload={handleTrackDownload}
        onTrackContextMenu={(track: AudioInfo) => {
          window.electronAPI.showTrackContextMenu(track);
        }}
        onWorkspaceChange={handleWorkspaceChange}
        onToggleLikedFilter={handleToggleLikedFilter}
        likedFilter={libraryState.likedFilter}
      />

      {notification && <Notification message={notification} />}
    </div>
  );
};

export default App;
