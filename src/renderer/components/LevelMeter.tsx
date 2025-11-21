import React, { useRef, useEffect, useState } from 'react';
import './LevelMeter.css';

interface LevelMeterProps {
  level: number; // RMS level 0-1
  orientation?: 'vertical' | 'horizontal';
  height?: number;
  width?: number;
}

const LevelMeter: React.FC<LevelMeterProps> = ({
  level,
  orientation = 'vertical',
  height = 80,
  width = 12,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  // Convert RMS to dB with gain compensation
  // Apply +18dB gain to match typical DJ mixer input levels
  const rmsToDb = (rms: number): number => {
    if (rms <= 0) return -Infinity;
    const dbfs = 20 * Math.log10(rms);
    return dbfs + 18; // Add gain to bring typical music levels into meter range
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const isVertical = orientation === 'vertical';
    const w = isVertical ? width : height;
    const h = isVertical ? height : width;
    canvas.width = w;
    canvas.height = h;

    // Clear
    ctx.clearRect(0, 0, w, h);

    const currentDb = rmsToDb(level);

    // Pioneer-style LED segments: 15 segments total
    // Top 2: Red (above +10dB, above +13dB)
    // Next 4: Orange 
    // Bottom 9: Green (lowest is -24dB)
    const numSegments = 15;
    const segmentGap = 1; // px gap between segments

    const drawLedSegments = (level: number) => {
      for (let i = 0; i < numSegments; i++) {
        // Calculate dB value for this segment
        // Segment 0 (bottom) = -24dB, Segment 14 (top) = +13dB
        const segmentDb = -24 + ((i / (numSegments - 1)) * (13 - (-24)));
        
        // Check if this segment should be lit
        const isLit = currentDb >= segmentDb;
        
        if (!isLit) continue;
        
        // Determine color based on segment index (from bottom)
        let color: string;
        if (i >= 13) {
          // Top 2 segments: Red (+10dB and above)
          color = '#ff0000';
        } else if (i >= 9) {
          // Next 4 segments: Orange
          color = '#ff8800';
        } else {
          // Bottom 9 segments: Green
          color = '#00ff00';
        }
        
        ctx.fillStyle = color;
        
        if (isVertical) {
          const segHeight = (h / numSegments) - segmentGap;
          const y = h - ((i + 1) / numSegments) * h;
          ctx.fillRect(0, y, w, segHeight);
        } else {
          const segWidth = (w / numSegments) - segmentGap;
          const x = (i / numSegments) * w;
          ctx.fillRect(x, 0, segWidth, h);
        }
      }
    };

    // Draw LED segments
    drawLedSegments(currentDb);
  }, [level, orientation, height, width]);

  return (
    <div className="level-meter-container">
      <canvas
        ref={canvasRef}
        className="level-meter-canvas"
        style={{
          width: orientation === 'vertical' ? `${width}px` : `${height}px`,
          height: orientation === 'vertical' ? `${height}px` : `${width}px`,
        }}
      />
    </div>
  );
};

export default LevelMeter;
