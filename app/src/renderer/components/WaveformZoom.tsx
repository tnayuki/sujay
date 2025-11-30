import React, { useRef, useEffect } from 'react';
import './Waveform.css';

type WaveformArray = Float32Array | number[];

interface WaveformZoomProps {
  waveform: WaveformArray;
  progress: number; // 0-1
  duration: number; // Track duration in seconds
  height?: number;
}

const WaveformZoom: React.FC<WaveformZoomProps> = ({
  waveform,
  progress,
  duration,
  height = 80,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const VISIBLE_DURATION = 8; // seconds

  // Draw waveform with color split and progress line
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !waveform || waveform.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const width = canvas.width;
    const centerY = height / 2;
    ctx.clearRect(0, 0, width, canvas.height);

    const currentTime = progress * duration;

    // Calculate visible range
    const visibleDuration = Math.min(VISIBLE_DURATION, duration);
    const playbackPositionRatio = 0.3;
    let startTime = currentTime - visibleDuration * playbackPositionRatio;
    let endTime = startTime + visibleDuration;

    if (startTime < 0) {
      startTime = 0;
      endTime = visibleDuration;
    }
    if (endTime > duration) {
      endTime = duration;
      startTime = Math.max(0, duration - visibleDuration);
    }

    const startIndex = Math.floor((startTime / duration) * waveform.length);
    const endIndex = Math.ceil((endTime / duration) * waveform.length);
    const totalVisibleSamples = endIndex - startIndex;

    const step = Math.max(1, Math.floor(totalVisibleSamples / width));
    const barWidth = (width * step) / totalVisibleSamples;
    const progressX = Math.max(0, Math.min(width, ((currentTime - startTime) / (endTime - startTime)) * width));

    // Draw all bars with color split in single pass
    for (let i = startIndex; i < endIndex; i += step) {
      if (i < 0 || i >= waveform.length) continue;

      let maxAmplitude = Math.abs(waveform[i]);
      for (let j = 1; j < step && i + j < endIndex && i + j < waveform.length; j++) {
        maxAmplitude = Math.max(maxAmplitude, Math.abs(waveform[i + j]));
      }

      const x = ((i - startIndex) / totalVisibleSamples) * width;
      const barHeight = maxAmplitude * centerY * 0.9;
      
      // Choose color based on progress
      ctx.fillStyle = x < progressX ? '#4a9eff' : '#ddd';
      ctx.fillRect(x, centerY - barHeight, Math.max(barWidth - 1, 1), barHeight * 2);
    }

    // Draw progress line
    ctx.strokeStyle = '#fff';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(progressX, 0);
    ctx.lineTo(progressX, canvas.height);
    ctx.stroke();
  }, [waveform, progress, duration, height, VISIBLE_DURATION]);

  return (
    <div
      style={{
        position: 'relative',
        width: '100%',
        height: `${height}px`,
      }}
    >
      <canvas
        ref={canvasRef}
        width={2000}
        height={height}
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: '100%',
          height: '100%',
        }}
      />
    </div>
  );
};

export default WaveformZoom;
