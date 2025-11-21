import React, { useRef, useEffect } from 'react';
import './Waveform.css';

interface WaveformFullProps {
  waveform: number[];
  progress: number; // 0-1
  height?: number;
  onSeek?: (position: number) => void;
}

const WaveformFull: React.FC<WaveformFullProps> = ({
  waveform,
  progress,
  height = 40,
  onSeek,
}) => {
  const backgroundCanvasRef = useRef<HTMLCanvasElement>(null);
  const progressCanvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!onSeek) return;
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const clickPosition = x / rect.width;
    onSeek(clickPosition);
  };

  // Draw background waveform (gray) - only once or when waveform changes
  useEffect(() => {
    const canvas = backgroundCanvasRef.current;
    if (!canvas || !waveform || waveform.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const width = canvas.width;
    const centerY = height / 2;
    ctx.clearRect(0, 0, width, canvas.height);

    // Downsample to canvas width
    const step = Math.max(1, Math.floor(waveform.length / width));
    const barWidth = width / (waveform.length / step);

    // Draw white mask
    ctx.fillStyle = '#ffffff';
    for (let i = 0; i < waveform.length; i += step) {
      let maxAmplitude = Math.abs(waveform[i]);
      for (let j = 1; j < step && i + j < waveform.length; j++) {
        maxAmplitude = Math.max(maxAmplitude, Math.abs(waveform[i + j]));
      }

      const x = (i / waveform.length) * width;
      const barHeight = maxAmplitude * centerY * 0.9;
      ctx.fillRect(x, centerY - barHeight, Math.max(barWidth - 1, 1), barHeight * 2);
    }

    // Apply gray tint
    ctx.globalCompositeOperation = 'source-in';
    ctx.fillStyle = '#ddd';
    ctx.fillRect(0, 0, width, canvas.height);
    ctx.globalCompositeOperation = 'source-over';
  }, [waveform, height]);

  // Draw progress overlay - updates with progress
  useEffect(() => {
    const canvas = progressCanvasRef.current;
    if (!canvas || !waveform || waveform.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const width = canvas.width;
    const centerY = height / 2;
    ctx.clearRect(0, 0, width, canvas.height);

    const progressX = progress * width;

    // Downsample to canvas width
    const step = Math.max(1, Math.floor(waveform.length / width));
    const barWidth = width / (waveform.length / step);

    // Draw white mask
    ctx.fillStyle = '#ffffff';
    for (let i = 0; i < waveform.length; i += step) {
      let maxAmplitude = Math.abs(waveform[i]);
      for (let j = 1; j < step && i + j < waveform.length; j++) {
        maxAmplitude = Math.max(maxAmplitude, Math.abs(waveform[i + j]));
      }

      const x = (i / waveform.length) * width;
      const barHeight = maxAmplitude * centerY * 0.9;
      ctx.fillRect(x, centerY - barHeight, Math.max(barWidth - 1, 1), barHeight * 2);
    }

    // Apply blue tint only to played portion
    ctx.globalCompositeOperation = 'source-in';
    ctx.fillStyle = '#4a9eff';
    ctx.fillRect(0, 0, progressX, canvas.height);
    ctx.globalCompositeOperation = 'source-over';

    // Draw progress line
    ctx.strokeStyle = '#fff';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(progressX, 0);
    ctx.lineTo(progressX, canvas.height);
    ctx.stroke();
  }, [waveform, progress, height]);

  return (
    <div
      ref={containerRef}
      onClick={handleClick}
      style={{
        position: 'relative',
        width: '100%',
        height: `${height}px`,
        cursor: onSeek ? 'pointer' : 'default',
      }}
    >
      <canvas
        ref={backgroundCanvasRef}
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
      <canvas
        ref={progressCanvasRef}
        width={2000}
        height={height}
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: '100%',
          height: '100%',
          pointerEvents: 'none',
        }}
      />
    </div>
  );
};

export default WaveformFull;
