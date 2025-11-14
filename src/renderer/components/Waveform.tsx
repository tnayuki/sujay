import React, { useRef, useEffect } from 'react';
import './Waveform.css';

interface WaveformProps {
  waveform: number[];
  progress: number; // 0-1
  duration: number; // Track duration in seconds
  height?: number;
  showFullWaveform?: boolean; // 全体波形を表示するか
  onSeek?: (position: number) => void; // シーク用コールバック
}

const Waveform: React.FC<WaveformProps> = ({
  waveform,
  progress,
  duration,
  height = 60,
  showFullWaveform = false,
  onSeek,
}) => {
  const waveformCanvasRef = useRef<HTMLCanvasElement>(null);
  const progressCanvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Fixed zoom: 4 bars ≈ 8 seconds at 120 BPM
  const VISIBLE_DURATION = 8; // seconds

  // クリックでシーク
  const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!onSeek || !showFullWaveform) return;

    const container = containerRef.current;
    if (!container) return;

    const rect = container.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const clickPosition = x / rect.width; // 表示サイズで割る（0-1）

    onSeek(clickPosition);
  };

  // Draw static waveform bars (only when waveform changes)
  useEffect(() => {
    const canvas = waveformCanvasRef.current;
    if (!canvas || !waveform || waveform.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const width = canvas.width;
    const centerY = height / 2;

    // Clear canvas
    ctx.clearRect(0, 0, width, height);

    const currentTime = progress * duration;

    let startIndex: number;
    let endIndex: number;
    let totalVisibleSamples: number;
    let startTime: number;
    let endTime: number;

    if (showFullWaveform) {
      // 全体波形を表示
      startIndex = 0;
      endIndex = waveform.length;
      totalVisibleSamples = waveform.length;
      startTime = 0;
      endTime = duration;
    } else {
      // ズームされた波形（32秒分）を表示
      const visibleDuration = Math.min(VISIBLE_DURATION, duration);

      // Position playback at 30% from left (like DJ software)
      const playbackPositionRatio = 0.3;
      startTime = currentTime - visibleDuration * playbackPositionRatio;
      endTime = startTime + visibleDuration;

      // Clamp to track boundaries
      if (startTime < 0) {
        startTime = 0;
        endTime = visibleDuration;
      }
      if (endTime > duration) {
        endTime = duration;
        startTime = Math.max(0, duration - visibleDuration);
      }

      // Convert time to waveform indices
      startIndex = Math.floor((startTime / duration) * waveform.length);
      endIndex = Math.ceil((endTime / duration) * waveform.length);
      totalVisibleSamples = endIndex - startIndex;
    }

    // Downsample if too many samples to draw (performance optimization)
    const maxBarsToRender = width; // Max 1 bar per pixel
    const step = Math.max(1, Math.floor(totalVisibleSamples / maxBarsToRender));
    const barWidth = (width * step) / totalVisibleSamples;

  // Draw all bars in light gray (static, no color based on progress)
  ctx.fillStyle = '#ddd';
    for (let i = startIndex; i < endIndex; i += step) {
      if (i < 0 || i >= waveform.length) continue;

      // Find max amplitude in this step range for better visual representation
      let maxAmplitude = Math.abs(waveform[i]);
      for (let j = 1; j < step && i + j < endIndex && i + j < waveform.length; j++) {
        maxAmplitude = Math.max(maxAmplitude, Math.abs(waveform[i + j]));
      }

      const x = ((i - startIndex) / totalVisibleSamples) * width;
      const barHeight = maxAmplitude * centerY * 0.9; // 90% of max height

      // Draw bar (centered vertically)
      ctx.fillRect(x, centerY - barHeight, Math.max(barWidth - 1, 1), barHeight * 2);
    }
  }, [waveform, progress, duration, height, showFullWaveform]);

  // Draw progress overlay (updates every frame)
  useEffect(() => {
    const canvas = progressCanvasRef.current;
    if (!canvas || !waveform || waveform.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const width = canvas.width;
    const centerY = height / 2;

    // Clear canvas
    ctx.clearRect(0, 0, width, height);

    const currentTime = progress * duration;
    const progressTime = progress * duration;

    let startIndex: number;
    let endIndex: number;
    let totalVisibleSamples: number;
    let startTime: number;
    let endTime: number;

    if (showFullWaveform) {
      // 全体波形を表示
      startIndex = 0;
      endIndex = waveform.length;
      totalVisibleSamples = waveform.length;
      startTime = 0;
      endTime = duration;
    } else {
      // ズームされた波形（32秒分）を表示
      const visibleDuration = Math.min(VISIBLE_DURATION, duration);

      // Position playback at 30% from left (like DJ software)
      const playbackPositionRatio = 0.3;
      startTime = currentTime - visibleDuration * playbackPositionRatio;
      endTime = startTime + visibleDuration;

      // Clamp to track boundaries
      if (startTime < 0) {
        startTime = 0;
        endTime = visibleDuration;
      }
      if (endTime > duration) {
        endTime = duration;
        startTime = Math.max(0, duration - visibleDuration);
      }

      // Convert time to waveform indices
      startIndex = Math.floor((startTime / duration) * waveform.length);
      endIndex = Math.ceil((endTime / duration) * waveform.length);
      totalVisibleSamples = endIndex - startIndex;
    }

    // Downsample if too many samples to draw (performance optimization)
    const maxBarsToRender = width; // Max 1 bar per pixel
    const step = Math.max(1, Math.floor(totalVisibleSamples / maxBarsToRender));
    const barWidth = (width * step) / totalVisibleSamples;

    // 1) Draw a white mask of the waveform bars over the full visible window
    ctx.globalCompositeOperation = 'source-over';
    ctx.fillStyle = '#ffffff';
    for (let i = startIndex; i < endIndex; i += step) {
      if (i < 0 || i >= waveform.length) continue;

      // Find max amplitude in this step range
      let maxAmplitude = Math.abs(waveform[i]);
      for (let j = 1; j < step && i + j < endIndex && i + j < waveform.length; j++) {
        maxAmplitude = Math.max(maxAmplitude, Math.abs(waveform[i + j]));
      }

      const x = ((i - startIndex) / totalVisibleSamples) * width;
      const barHeight = maxAmplitude * centerY * 0.9;
      ctx.fillRect(x, centerY - barHeight, Math.max(barWidth - 1, 1), barHeight * 2);
    }

    // 2) Apply blue tint only to the played portion using compositing
    const clampedProgressX = Math.max(0, Math.min(width, ((currentTime - startTime) / (endTime - startTime)) * width));
    ctx.globalCompositeOperation = 'source-in';
    ctx.fillStyle = '#4a9eff';
    ctx.fillRect(0, 0, clampedProgressX, height);

    // 3) Reset composite mode
    ctx.globalCompositeOperation = 'source-over';

    // Draw progress line
    const progressX = clampedProgressX;
    ctx.strokeStyle = '#fff';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(progressX, 0);
    ctx.lineTo(progressX, height);
    ctx.stroke();
  }, [waveform, progress, duration, height, showFullWaveform]);

  return (
    <div
      ref={containerRef}
      onClick={handleClick}
      style={{
        position: 'relative',
        width: '100%',
        height: `${height}px`,
        cursor: showFullWaveform && onSeek ? 'pointer' : 'default',
      }}
    >
      {/* Background layer: static waveform bars */}
      <canvas
        ref={waveformCanvasRef}
        width={2000}
        height={height}
        className="waveform-canvas waveform-canvas--background"
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: '100%',
          height: '100%',
        }}
      />
      {/* Foreground layer: progress overlay */}
      <canvas
        ref={progressCanvasRef}
        width={2000}
        height={height}
        className="waveform-canvas"
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: '100%',
          height: '100%',
          pointerEvents: 'none', // Allow clicks to pass through
        }}
      />
    </div>
  );
};

export default Waveform;
