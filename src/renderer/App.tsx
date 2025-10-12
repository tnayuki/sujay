import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import type { AudioEngineState, LibraryState, Track, Workspace } from '../types';
import type { AudioInfo } from '../suno-api';
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
  });


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

  // Use ref to prevent simultaneous track loading (which causes ffmpeg conflicts)
  const isLoadingTrackRef = useRef(false);

  // Initialize states and event listeners
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

    const handleAudioStateChanged = (state: AudioEngineState) => {
      if (mounted) {
        setAudioState(state);
      }
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
      }, 500); // Throttle to 500ms
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

    const handleLibrarySyncStarted = (data: any) => {
      if (mounted) {
        setSyncStatus({ syncing: true });
      }
    };

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

    const handleLibrarySyncCompleted = (data: any) => {
      if (mounted) {
        setSyncStatus({ syncing: false });
      }
    };

    const handleLibrarySyncFailed = (data: any) => {
      if (mounted) {
        setSyncStatus({ syncing: false });
      }
    };


    const unsubscribeAudio = window.electronAPI.onAudioStateChanged(handleAudioStateChanged);
    const unsubscribeLibrary = window.electronAPI.onLibraryStateChanged(handleLibraryStateChanged);
    const unsubscribeProgress = window.electronAPI.onDownloadProgressChanged(handleDownloadProgressChanged);
    const unsubscribeNotification = window.electronAPI.onNotification(handleNotification);
    const unsubscribeSyncStarted = window.electronAPI.onLibrarySyncStarted(handleLibrarySyncStarted);
    const unsubscribeSyncProgress = window.electronAPI.onLibrarySyncProgress(handleLibrarySyncProgress);
    const unsubscribeSyncCompleted = window.electronAPI.onLibrarySyncCompleted(handleLibrarySyncCompleted);
    const unsubscribeSyncFailed = window.electronAPI.onLibrarySyncFailed(handleLibrarySyncFailed);


    // Cleanup function
    return () => {
      mounted = false;
      if (notificationTimer) {
        clearTimeout(notificationTimer);
      }
      if (downloadProgressTimeout) {
        clearTimeout(downloadProgressTimeout);
      }
      // Remove event listeners
      unsubscribeAudio();
      unsubscribeLibrary();
      unsubscribeProgress();
      unsubscribeNotification();
      unsubscribeSyncStarted();
      unsubscribeSyncProgress();
      unsubscribeSyncCompleted();
      unsubscribeSyncFailed();
    };
  }, []);

  const handleTrackClick = useCallback(async (audioInfo: AudioInfo) => {
    // Prevent simultaneous track loading (causes ffmpeg conflicts)
    if (isLoadingTrackRef.current) {
      console.log('Already loading a track, please wait...');
      return;
    }

    isLoadingTrackRef.current = true;

    try {
      // Download the track first to get the local cache path
      const downloadedTrack = await window.electronAPI.libraryDownloadTrack(audioInfo);

      // Play the downloaded track (simple playback)
      await window.electronAPI.audioPlay(downloadedTrack);
    } catch (error) {
      console.error('Error in handleTrackClick:', error);
    } finally {
      isLoadingTrackRef.current = false;
    }
  }, []);

  const handleTrackDownload = useCallback(async (audioInfo: AudioInfo) => {
    try {
      // Just download the track, don't play it
      await window.electronAPI.libraryDownloadTrack(audioInfo);
    } catch (error) {
      console.error('Error downloading track:', error);
    }
  }, []);


  const handleWorkspaceChange = useCallback((workspace: Workspace | null) => {
    window.electronAPI.librarySetWorkspace(workspace);
  }, []);

  return (
    <div className="app">
      <Library
        tracks={libraryState.tracks}
        workspaces={libraryState.workspaces}
        currentWorkspace={libraryState.selectedWorkspace}
        syncStatus={syncStatus}
        downloadProgress={downloadProgress}
        currentPlayingTrackId={audioState.isPlaying ? (audioState.currentTrack?.id || null) : null}
        onTrackClick={handleTrackClick}
        onTrackDownload={handleTrackDownload}
        onWorkspaceChange={handleWorkspaceChange}
      />

      {notification && <Notification message={notification} />}
    </div>
  );
};

export default App;
