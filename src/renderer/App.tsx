import React, { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import type { AudioEngineState, LibraryState, Track, Workspace } from '../types';
import type { AudioInfo } from '../suno-api';
import Console from './components/Console';
import Library from './components/Library';
import Notification from './components/Notification';
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
  });

  type WaveformBuffer = {
    chunks: (number[] | null)[];
    totalChunks: number;
  };

  const deckAWaveformRef = useRef<number[] | null>(null);
  const deckBWaveformRef = useRef<number[] | null>(null);
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

  useEffect(() => {
    let mounted = true;

    const initializeStates = async () => {
      const audio = await window.electronAPI.audioGetState();
      const library = await window.electronAPI.libraryGetState();
      const progress = await window.electronAPI.libraryGetDownloadProgress();

      if (mounted) {
        setAudioState(audio);
        setLibraryState(library);
        setDownloadProgress(new Map(progress));
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
      if (!state.deckA || prevDeckAId !== newDeckAId) {
        deckAWaveformRef.current = null;
        if (prevDeckAId && prevDeckAId !== newDeckBId) {
          delete waveformBuffersRef.current[prevDeckAId];
        }
      }
      if (!state.deckB || prevDeckBId !== newDeckBId) {
        deckBWaveformRef.current = null;
        if (prevDeckBId && prevDeckBId !== newDeckAId) {
          delete waveformBuffersRef.current[prevDeckBId];
        }
      }

      const cleanedState: AudioEngineState = {
        ...state,
        deckA: stripWaveformData(state.deckA || null),
        deckB: stripWaveformData(state.deckB || null),
        currentTrack: stripWaveformData(state.currentTrack || null) || undefined,
        nextTrack: stripWaveformData(state.nextTrack || null) || undefined,
      };

      setAudioState(cleanedState);
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
    const handleLibrarySyncProgress = (data: any) => {
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

    const unsubscribeAudio = window.electronAPI.onAudioStateChanged(handleAudioStateChanged);
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
        await window.electronAPI.audioPlay(downloadedTrack, false, data.deck);
      } catch (error) {
        console.error('Error handling track load deck event:', error);
      } finally {
        isLoadingTrackRef.current = false;
      }
    });
    const unsubscribeWaveformChunk = window.electronAPI.onWaveformChunk(handleWaveformChunk);
    const unsubscribeWaveformComplete = window.electronAPI.onWaveformComplete(handleWaveformComplete);

    return () => {
      mounted = false;
      if (notificationTimer) {
        clearTimeout(notificationTimer);
      }
      if (downloadProgressTimeout) {
        clearTimeout(downloadProgressTimeout);
      }
      unsubscribeAudio();
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

  const handleWorkspaceChange = useCallback((workspace: Workspace | null) => {
    window.electronAPI.librarySetWorkspace(workspace);
  }, []);

  const handleToggleLikedFilter = useCallback(() => {
    window.electronAPI.libraryToggleLikedFilter();
  }, []);

  const activeTrackIds = [audioState.deckA?.id, audioState.deckB?.id].filter((id): id is string => Boolean(id));

  const currentTrackWithWaveform = useMemo(() => {
    if (!audioState.deckA) return null;
    return {
      ...audioState.deckA,
      waveformData: deckAWaveformRef.current || undefined,
    };
  }, [audioState.deckA, waveformVersion]);

  const nextTrackWithWaveform = useMemo(() => {
    if (!audioState.deckB) return null;
    return {
      ...audioState.deckB,
      waveformData: deckBWaveformRef.current || undefined,
    };
  }, [audioState.deckB, waveformVersion]);

  return (
    <div className="app">
      <div className="titlebar-overlay">
        <div className="titlebar-title">Sujay</div>
        <div className="titlebar-info">
          <span className="time">{systemInfo.time}</span>
          <span className="cpu-label">CPU</span>
          <div className="cpu-bar">
            <div 
              className="cpu-bar-fill" 
              style={{ width: `${Math.min(100, systemInfo.cpuUsage)}%` }}
            ></div>
          </div>
          <span className="cpu-value">{systemInfo.cpuUsage.toFixed(1)}%</span>
        </div>
      </div>
      <Console
        currentTrack={currentTrackWithWaveform}
        nextTrack={nextTrackWithWaveform}
        position={audioState.deckAPosition || 0}
        nextPosition={audioState.deckBPosition || 0}
        deckAPlaying={audioState.deckAPlaying}
        deckBPlaying={audioState.deckBPlaying}
        isPlaying={audioState.isPlaying}
        isCrossfading={audioState.isCrossfading}
        crossfadeProgress={audioState.crossfadeProgress}
        crossfaderPosition={audioState.crossfaderPosition}
        masterTempo={audioState.masterTempo}
        onStop={handleStopClick}
        onSeek={handleSeek}
        onCrossfaderChange={handleCrossfaderChange}
        onMasterTempoChange={handleMasterTempoChange}
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
