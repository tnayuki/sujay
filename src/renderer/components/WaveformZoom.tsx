import React, { useRef, useEffect } from 'react';
import './Waveform.css';

interface WaveformZoomProps {
  waveform: number[];
  progress: number; // 0-1
  duration: number; // Track duration in seconds
  isPlaying: boolean; // Whether the track is currently playing
  bpm?: number; // Track BPM
  masterTempo?: number; // Master tempo for rate calculation
  height?: number;
  isSeek?: boolean; // Whether the position update is from seek
}

const WaveformZoom: React.FC<WaveformZoomProps> = ({
  waveform,
  progress,
  duration,
  isPlaying,
  bpm,
  masterTempo = 130,
  height = 80,
  isSeek,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animationFrameRef = useRef<number>();
  const lastRenderTimeRef = useRef<number>(0);
  const lastProgressRef = useRef<number>(progress);
  const lastProgressUpdateTimeRef = useRef<number>(performance.now());

  const VISIBLE_DURATION = 8; // seconds
  const FPS_LIMIT = 30;

  // Update base progress when it changes from seek operations only
  useEffect(() => {
    if (isSeek) {
      lastProgressRef.current = progress;
      lastProgressUpdateTimeRef.current = performance.now();
    }
  }, [progress, isSeek]);

  // Draw waveform with color split and progress line - single pass rendering
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !waveform || waveform.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const render = (timestamp: number) => {
      const timeSinceLastRender = timestamp - lastRenderTimeRef.current;
      const frameInterval = 1000 / FPS_LIMIT;

      if (timeSinceLastRender < frameInterval) {
        animationFrameRef.current = requestAnimationFrame(render);
        return;
      }

      lastRenderTimeRef.current = timestamp;

      const width = canvas.width;
      const centerY = height / 2;
      ctx.clearRect(0, 0, width, canvas.height);

      // Calculate current position based on playback state
      let interpolatedProgress: number;
      if (isPlaying && bpm && masterTempo) {
        // Calculate playback rate (tempo adjustment)
        const rate = masterTempo / bpm;
        // Calculate elapsed time since last position update
        const timeSinceUpdate = (timestamp - lastProgressUpdateTimeRef.current) / 1000; // seconds
        // Calculate how far we've progressed (in seconds of audio)
        const progressedSeconds = timeSinceUpdate * rate;
        // Convert to progress ratio
        interpolatedProgress = Math.min(1, lastProgressRef.current + (progressedSeconds / duration));
      } else {
        // Not playing or no BPM info, use last known position
        interpolatedProgress = lastProgressRef.current;
      }
      
      const currentTime = interpolatedProgress * duration;

      // Calculate visible range based on interpolated position
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

      animationFrameRef.current = requestAnimationFrame(render);
    };

    animationFrameRef.current = requestAnimationFrame(render);

    return () => {
      if (animationFrameRef.current) {
        cancelAnimationFrame(animationFrameRef.current);
      }
    };
  }, [waveform, progress, duration, height, FPS_LIMIT, VISIBLE_DURATION, isPlaying, bpm, masterTempo]);

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
