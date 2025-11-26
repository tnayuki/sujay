import React, { useState, useRef, useCallback, useEffect } from 'react';
import type { Track } from '../../types';
import WaveformFull from './WaveformFull';
import WaveformZoom from './WaveformZoom';
import LevelMeter from './LevelMeter';
import './Console.css';

interface ConsoleProps {
  currentTrack: Track | null;
  nextTrack: Track | null;
  position: number;
  nextPosition: number;
  isSeek?: boolean;
  deckAPlaying: boolean;
  deckBPlaying: boolean;
  deckAPeak: number;
  deckBPeak: number;
  deckAPeakHold: number;
  deckBPeakHold: number;
  deckACueEnabled: boolean;
  deckBCueEnabled: boolean;
  isPlaying: boolean;
  isCrossfading: boolean;
  crossfadeProgress: number;
  crossfaderPosition: number;
  masterTempo: number;
  onStop: (deck: 1 | 2) => void;
  onSeek: (deck: 1 | 2, position: number) => void;
  onCrossfaderChange: (position: number) => void;
  onMasterTempoChange: (bpm: number) => void;
  onDeckCueToggle: (deck: 1 | 2, enabled: boolean) => void;
  onPlay: (deck: 1 | 2) => void;
}

const formatTime = (seconds: number): string => {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, '0')}`;
};

const HeadphoneIcon: React.FC = () => (
  <svg viewBox="0 0 24 24" role="presentation" focusable="false" aria-hidden="true">
    <path
      d="M4 14v4a2 2 0 0 0 2 2h1v-6H6a2 2 0 0 0-2 2zm13-2c0-3.866-3.134-7-7-7s-7 3.134-7 7"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
    />
    <path
      d="M20 16a2 2 0 0 0-2-2h-1v6h1a2 2 0 0 0 2-2v-2z"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
    />
    <path
      d="M18 14v-2c0-3.866-3.134-7-7-7"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
    />
  </svg>
);

const Console: React.FC<ConsoleProps> = ({
  currentTrack,
  nextTrack,
  position,
  nextPosition,
  isSeek,
  deckAPlaying,
  deckBPlaying,
  deckAPeak,
  deckBPeak,
  deckAPeakHold,
  deckBPeakHold,
  deckACueEnabled,
  deckBCueEnabled,
  crossfaderPosition,
  masterTempo,
  onStop,
  onSeek,
  onCrossfaderChange,
  onMasterTempoChange,
  onDeckCueToggle,
  onPlay,
}) => {
  const [isDragging, setIsDragging] = useState(false);
  const [tempoInputValue, setTempoInputValue] = useState(masterTempo.toString());
  const crossfaderRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const tempoInputValueRef = useRef(tempoInputValue);
  const isChangingTempoRef = useRef(false);

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

  // Sync input value only when not focused
  useEffect(() => {
    tempoInputValueRef.current = tempoInputValue;
  }, [tempoInputValue]);

  useEffect(() => {
    if (document.activeElement !== inputRef.current) {
      const nextValue = masterTempo.toString();
      if (tempoInputValueRef.current === nextValue) {
        return;
      }
      if (isChangingTempoRef.current) {
        const expected = parseFloat(tempoInputValueRef.current);
        if (!isNaN(expected) && Math.abs(masterTempo - expected) > 0.01) {
          return;
        }
        isChangingTempoRef.current = false;
      }
      tempoInputValueRef.current = nextValue;
      setTempoInputValue(nextValue);
    }
  }, [masterTempo]);

  const handleTempoInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    setTempoInputValue(e.target.value);
    tempoInputValueRef.current = e.target.value;
  }, []);

  const commitTempoChange = useCallback(() => {
    const bpm = parseFloat(tempoInputValueRef.current);
    if (!isNaN(bpm) && bpm > 0 && bpm <= 300) {
      if (Math.abs(bpm - masterTempo) < 0.01) {
        isChangingTempoRef.current = false;
        const resetValue = masterTempo.toString();
        tempoInputValueRef.current = resetValue;
        setTempoInputValue(resetValue);
        return;
      }
      isChangingTempoRef.current = true;
      onMasterTempoChange(bpm);
    } else {
      isChangingTempoRef.current = false;
      const resetValue = masterTempo.toString();
      tempoInputValueRef.current = resetValue;
      setTempoInputValue(resetValue);
    }
  }, [masterTempo, onMasterTempoChange]);

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
    const currentValue = parseFloat(tempoInputValueRef.current);
    const baseValue = isNaN(currentValue) ? masterTempo : currentValue;
    const newTempo = baseValue + delta;
    if (newTempo > 0 && newTempo <= 300) {
      if (Math.abs(newTempo - masterTempo) < 0.01) {
        isChangingTempoRef.current = false;
        const resetValue = masterTempo.toString();
        tempoInputValueRef.current = resetValue;
        setTempoInputValue(resetValue);
        return;
      }
      isChangingTempoRef.current = true;
      const nextValue = newTempo.toString();
      setTempoInputValue(nextValue);
      tempoInputValueRef.current = nextValue;
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
            <WaveformZoom
              waveform={currentTrack.waveformData}
              progress={position / currentTrack.duration}
              duration={currentTrack.duration}
              isPlaying={deckAPlaying}
              bpm={currentTrack.bpm}
              masterTempo={masterTempo}
              height={80}
              isSeek={isSeek}
            />
          ) : (
            <div style={{ height: '80px', backgroundColor: '#333' }} />
          )}
        </div>

        {/* Deck 2 Waveform */}
        <div className="waveform-deck">
          {nextTrack && nextTrack.waveformData && nextTrack.waveformData.length > 0 ? (
            <WaveformZoom
              waveform={nextTrack.waveformData}
              progress={nextPosition / nextTrack.duration}
              duration={nextTrack.duration}
              isPlaying={deckBPlaying}
              bpm={nextTrack.bpm}
              masterTempo={masterTempo}
              height={80}
              isSeek={isSeek}
            />
          ) : (
            <div style={{ height: '80px', backgroundColor: '#333' }} />
          )}
        </div>
      </div>

      {/* Deck Info and Controls */}
      <div className="decks">
        {/* Deck 1 (Left) - Current Track */}
        <div className={`deck deck-left ${currentTrack ? 'active' : 'inactive'}`}>
          <div className="deck-header">
            <div className="deck-label">1</div>
            {currentTrack?.cachedImageData ? (
              <img src={currentTrack.cachedImageData} alt={currentTrack.title} className="deck-thumbnail" draggable={false} />
            ) : (
              <div className="deck-thumbnail-placeholder">🎵</div>
            )}
            <div className="deck-info-inline">
              <div className="deck-title">{currentTrack?.title || 'No track loaded'}</div>
              <div className="deck-time">
                {currentTrack ? (
                  <>
                    {formatTime(position)} / {formatTime(currentTrack.duration)}
                    {currentTrack.bpm && (
                      <span className="deck-bpm"> • {Math.round(currentTrack.bpm)} BPM</span>
                    )}
                  </>
                ) : (
                  '--:-- / --:--'
                )}
              </div>
            </div>
            {deckAPlaying ? (
              <button onClick={() => onStop(1)} className="deck-stop-button" title="Stop">
                ■
              </button>
            ) : (
              <button onClick={() => onPlay(1)} className="deck-play-button" title="Play" disabled={!currentTrack}>
                ▶
              </button>
            )}
          </div>
          <div className="deck-info">
            <div className="deck-waveform-full">
              {currentTrack?.waveformData && currentTrack.waveformData.length > 0 && (
                <WaveformFull
                  waveform={currentTrack.waveformData}
                  progress={position / currentTrack.duration}
                  height={50}
                  onSeek={(pos: number) => onSeek(1, pos)}
                />
              )}
            </div>
          </div>
        </div>

        {/* Tempo Control (Middle) */}
        <div className="tempo-section">
          <div className="tempo-controls">
            <button onClick={() => handleTempoAdjust(1)} className="tempo-button">
              ▲
            </button>
            <input
              ref={inputRef}
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
          <div className="level-meters">
            <div className="level-meter-column">
              <LevelMeter 
                peak={deckAPeak}
                peakHold={deckAPeakHold}
                orientation="vertical" 
                height={60} 
                width={10} 
              />
              <button
                type="button"
                className={`deck-cue-toggle under-meter ${deckACueEnabled ? 'active' : ''}`}
                onClick={() => onDeckCueToggle(1, !deckACueEnabled)}
                title="Deck A をキューに送る"
                aria-label="Deck A cue"
                aria-pressed={deckACueEnabled}
              >
                <span className="deck-cue-icon" aria-hidden="true">
                  <HeadphoneIcon />
                </span>
              </button>
            </div>
            <div className="level-meter-column">
              <LevelMeter 
                peak={deckBPeak}
                peakHold={deckBPeakHold}
                orientation="vertical" 
                height={60} 
                width={10} 
              />
              <button
                type="button"
                className={`deck-cue-toggle under-meter ${deckBCueEnabled ? 'active' : ''}`}
                onClick={() => onDeckCueToggle(2, !deckBCueEnabled)}
                title="Deck B をキューに送る"
                aria-label="Deck B cue"
                aria-pressed={deckBCueEnabled}
              >
                <span className="deck-cue-icon" aria-hidden="true">
                  <HeadphoneIcon />
                </span>
              </button>
            </div>
          </div>
        </div>

        {/* Deck 2 (Right) */}
        <div className={`deck deck-right ${nextTrack ? 'active' : 'inactive'}`}>
          <div className="deck-header">
            {deckBPlaying ? (
              <button onClick={() => onStop(2)} className="deck-stop-button" title="Stop">
                ■
              </button>
            ) : (
              <button onClick={() => onPlay(2)} className="deck-play-button" title="Play" disabled={!nextTrack}>
                ▶
              </button>
            )}
            <div className="deck-info-inline">
              <div className="deck-title">{nextTrack?.title || 'No track loaded'}</div>
              <div className="deck-time">
                {nextTrack ? (
                  <>
                    {formatTime(nextPosition)} / {formatTime(nextTrack.duration)}
                    {nextTrack.bpm && (
                      <span className="deck-bpm"> • {Math.round(nextTrack.bpm)} BPM</span>
                    )}
                  </>
                ) : (
                  '--:-- / --:--'
                )}
              </div>
            </div>
            {nextTrack?.cachedImageData ? (
              <img src={nextTrack.cachedImageData} alt={nextTrack.title} className="deck-thumbnail" draggable={false} />
            ) : (
              <div className="deck-thumbnail-placeholder">🎵</div>
            )}
            <div className="deck-label">2</div>
          </div>
          <div className="deck-info">
            <div className="deck-waveform-full">
              {nextTrack?.waveformData && nextTrack.waveformData.length > 0 && (
                <WaveformFull
                  waveform={nextTrack.waveformData}
                  progress={nextPosition / nextTrack.duration}
                  height={50}
                  onSeek={(pos: number) => onSeek(2, pos)}
                />
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Crossfader */}
      <div className="crossfader">
        <div className="crossfader-track-wrapper">
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
    </div>
  );
};

export default Console;
