import React, { useState, useRef, useCallback, useEffect } from 'react';
import type { Track } from '../../types';
import Waveform from './Waveform';
import './Console.css';

interface ConsoleProps {
  currentTrack: Track | null;
  nextTrack: Track | null;
  position: number;
  nextPosition: number;
  deckAPlaying: boolean;
  deckBPlaying: boolean;
  isPlaying: boolean;
  isCrossfading: boolean;
  crossfadeProgress: number;
  crossfaderPosition: number;
  masterTempo: number;
  onStop: (deck: 1 | 2) => void;
  onSeek: (deck: 1 | 2, position: number) => void;
  onCrossfaderChange: (position: number) => void;
  onMasterTempoChange: (bpm: number) => void;
  onPlay: (deck: 1 | 2) => void;
}

const formatTime = (seconds: number): string => {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, '0')}`;
};

const Console: React.FC<ConsoleProps> = ({
  currentTrack,
  nextTrack,
  position,
  nextPosition,
  deckAPlaying,
  deckBPlaying,
  isPlaying,
  isCrossfading,
  crossfadeProgress,
  crossfaderPosition,
  masterTempo,
  onStop,
  onSeek,
  onCrossfaderChange,
  onMasterTempoChange,
  onPlay,
}) => {
  const [isDragging, setIsDragging] = useState(false);
  const [tempoInputValue, setTempoInputValue] = useState(masterTempo.toString());
  const crossfaderRef = useRef<HTMLDivElement>(null);

  const updateCrossfaderPosition = useCallback((clientX: number) => {
    if (!crossfaderRef.current) return;

    const rect = crossfaderRef.current.getBoundingClientRect();
    const x = clientX - rect.left;
    const position = Math.max(0, Math.min(1, x / rect.width));
    onCrossfaderChange(position);
  }, [onCrossfaderChange]);

  // Handle crossfader drag
  const handleCrossfaderMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    updateCrossfaderPosition(e.clientX);
  }, [updateCrossfaderPosition]);

  const handleMouseMove = useCallback((e: MouseEvent) => {
    updateCrossfaderPosition(e.clientX);
  }, [updateCrossfaderPosition]);

  const handleMouseUp = useCallback(() => {
    setIsDragging(false);
  }, []);

  // Add/remove mouse event listeners for dragging
  useEffect(() => {
    if (isDragging) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
      return () => {
        document.removeEventListener('mousemove', handleMouseMove);
        document.removeEventListener('mouseup', handleMouseUp);
      };
    }
  }, [isDragging, handleMouseMove, handleMouseUp]);

  useEffect(() => {
    setTempoInputValue(masterTempo.toString());
  }, [masterTempo]);

  const handleTempoInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    setTempoInputValue(e.target.value);
  }, []);

  const commitTempoChange = useCallback(() => {
    const bpm = parseFloat(tempoInputValue);
    if (!isNaN(bpm) && bpm > 0 && bpm <= 300) {
      onMasterTempoChange(bpm);
    } else {
      setTempoInputValue(masterTempo.toString());
    }
  }, [tempoInputValue, masterTempo, onMasterTempoChange]);

  const handleTempoInputBlur = useCallback(() => {
    commitTempoChange();
  }, [commitTempoChange]);

  const handleTempoInputKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      commitTempoChange();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      handleTempoAdjust(1);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      handleTempoAdjust(-1);
    }
  }, [commitTempoChange]);

  const handleTempoAdjust = useCallback((delta: number) => {
    const newTempo = masterTempo + delta;
    if (newTempo > 0 && newTempo <= 300) {
      onMasterTempoChange(newTempo);
    }
  }, [masterTempo, onMasterTempoChange]);

  return (
    <div className="console">
      {/* Waveforms at the top */}
      <div className="waveforms">
        {/* Deck 1 Waveform */}
        <div className="waveform-deck">
          {currentTrack && currentTrack.waveformData && currentTrack.waveformData.length > 0 ? (
            <Waveform
              waveform={currentTrack.waveformData}
              progress={position / currentTrack.duration}
              duration={currentTrack.duration}
              height={80}
            />
          ) : (
            <div style={{ height: '80px', backgroundColor: '#333' }} />
          )}
        </div>

        {/* Deck 2 Waveform */}
        <div className="waveform-deck">
          {nextTrack && nextTrack.waveformData && nextTrack.waveformData.length > 0 ? (
            <Waveform
              waveform={nextTrack.waveformData}
              progress={nextPosition / nextTrack.duration}
              duration={nextTrack.duration}
              height={80}
            />
          ) : (
            <div style={{ height: '80px', backgroundColor: '#333' }} />
          )}
        </div>
      </div>

      {/* Deck Info and Controls */}
      <div className="decks">
        {/* Deck 1 (Left) - Current Track */}
        <div className={`deck ${currentTrack ? 'active' : 'inactive'}`}>
          <div className="deck-header">
            <div className="deck-label">1</div>
            {currentTrack && (
              <>
                <div className="deck-info-inline">
                  <div className="deck-title">{currentTrack.title}</div>
                  <div className="deck-time">
                    {formatTime(position)} / {formatTime(currentTrack.duration)}
                    {currentTrack.bpm && (
                      <span className="deck-bpm"> • {Math.round(currentTrack.bpm)} BPM</span>
                    )}
                  </div>
                </div>
                {deckAPlaying ? (
                  <button onClick={() => onStop(1)} className="deck-stop-button" title="Stop">
                    ■
                  </button>
                ) : (
                  <button onClick={() => onPlay(1)} className="deck-play-button" title="Play">
                    ▶
                  </button>
                )}
              </>
            )}
          </div>
          {currentTrack ? (
            <>
              {currentTrack.waveformData && currentTrack.waveformData.length > 0 ? (
                <div className="deck-info">
                  <div className="deck-waveform-full">
                    <Waveform
                      waveform={currentTrack.waveformData}
                      progress={position / currentTrack.duration}
                      duration={currentTrack.duration}
                      height={40}
                      showFullWaveform={true}
                      onSeek={(pos) => onSeek(1, pos)}
                    />
                  </div>
                </div>
              ) : (
                <div className="deck-progress">
                  <div
                    className="deck-progress-bar"
                    style={{ width: `${(position / currentTrack.duration) * 100}%` }}
                  />
                </div>
              )}
            </>
          ) : (
            <div className="deck-empty">No track loaded</div>
          )}
        </div>

        {/* Tempo Control (Middle) */}
        <div className="tempo-section">
          <div className="tempo-controls">
            <button onClick={() => handleTempoAdjust(1)} className="tempo-button">
              ▲
            </button>
            <input
              type="text"
              value={tempoInputValue}
              onChange={handleTempoInputChange}
              onBlur={handleTempoInputBlur}
              onKeyDown={handleTempoInputKeyDown}
              className="tempo-input"
            />
            <button onClick={() => handleTempoAdjust(-1)} className="tempo-button">
              ▼
            </button>
          </div>
        </div>

        {/* Deck 2 (Right) - Next Track */}
        <div className={`deck ${nextTrack ? 'active' : 'inactive'}`}>
          <div className="deck-header">
            <div className="deck-label">2</div>
            {nextTrack && (
              <>
                <div className="deck-info-inline">
                  <div className="deck-title">{nextTrack.title}</div>
                  <div className="deck-time">
                    {formatTime(nextPosition)} / {formatTime(nextTrack.duration)}
                    {nextTrack.bpm && (
                      <span className="deck-bpm"> • {Math.round(nextTrack.bpm)} BPM</span>
                    )}
                  </div>
                </div>
                {deckBPlaying ? (
                  <button onClick={() => onStop(2)} className="deck-stop-button" title="Stop">
                    ■
                  </button>
                ) : (
                  <button onClick={() => onPlay(2)} className="deck-play-button" title="Play">
                    ▶
                  </button>
                )}
              </>
            )}
          </div>
          {nextTrack ? (
            <>
              {nextTrack.waveformData && nextTrack.waveformData.length > 0 ? (
                <div className="deck-info">
                  <div className="deck-waveform-full">
                    <Waveform
                      waveform={nextTrack.waveformData}
                      progress={nextPosition / nextTrack.duration}
                      duration={nextTrack.duration}
                      height={40}
                      showFullWaveform={true}
                      onSeek={(pos) => onSeek(2, pos)}
                    />
                  </div>
                </div>
              ) : (
                <div className="deck-progress">
                  <div
                    className="deck-progress-bar"
                    style={{ width: `${(nextPosition / nextTrack.duration) * 100}%` }}
                  />
                </div>
              )}
            </>
          ) : (
            <div className="deck-empty">No track loaded</div>
          )}
        </div>
      </div>

      {/* Crossfader */}
      <div className="crossfader">
        <div
          className="crossfader-track"
          ref={crossfaderRef}
          onMouseDown={handleCrossfaderMouseDown}
          style={{ cursor: 'pointer' }}
        >
          <div className="crossfader-slider" style={{ left: `${crossfaderPosition * 100}%` }} />
        </div>
      </div>
    </div>
  );
};

export default Console;
