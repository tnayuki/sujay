import React, { useRef, useEffect } from 'react';
import './LevelMeter.css';

interface LevelMeterProps {
  peak: number; // Peak level 0-1
  peakHold?: number; // Peak hold level 0-1
  orientation?: 'vertical' | 'horizontal';
  height?: number;
  width?: number;
}

const LevelMeter: React.FC<LevelMeterProps> = ({
  peak,
  peakHold,
  orientation = 'vertical',
  height = 80,
  width = 12,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const MIN_DB = -24;
  const MAX_DB = 13;
  const PEAK_DISPLAY_OFFSET_DB = 8; // Approximate Pioneer calibration: 0dBFS -> +8dB meter

  const peakToDb = (peakValue: number): number => {
    if (peakValue <= 0) return -Infinity;
    const dbfs = 20 * Math.log10(peakValue);
    return Math.min(MAX_DB, dbfs + PEAK_DISPLAY_OFFSET_DB);
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

    const currentPeakValue = peak;
    const currentHoldValue = peakHold ?? 0;
    
    const currentDb = peakToDb(currentPeakValue);
    const holdDb = peakToDb(currentHoldValue);

    // Pioneer-style LED segments: 15 segments total
    // Top 2: Red (above +10dB, above +13dB)
    // Next 4: Orange 
    // Bottom 9: Green (lowest is -24dB)
    const numSegments = 15;
    const segmentGap = 1; // px gap between segments

    const getSegmentColor = (index: number): string => {
      if (index >= 13) return '#ff0000';
      if (index >= 9) return '#ff8800';
      return '#00ff00';
    };

    const drawPeakSegments = () => {
      for (let i = 0; i < numSegments; i++) {
        const segmentDb = MIN_DB + ((i / (numSegments - 1)) * (MAX_DB - MIN_DB));
        if (currentDb < segmentDb) continue;

        ctx.fillStyle = getSegmentColor(i);

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

    const drawHoldIndicator = () => {
      if (!Number.isFinite(holdDb) || holdDb === -Infinity) {
        return;
      }

      const normalized = (holdDb - MIN_DB) / (MAX_DB - MIN_DB);
      if (!Number.isFinite(normalized)) {
        return;
      }

      const holdIndex = Math.max(0, Math.min(numSegments - 1, Math.round(normalized * (numSegments - 1))));

      ctx.save();
      ctx.fillStyle = getSegmentColor(holdIndex);
      ctx.globalAlpha = 0.7;

      if (isVertical) {
        const segHeight = (h / numSegments) - segmentGap;
        const y = h - ((holdIndex + 1) / numSegments) * h;
        ctx.fillRect(0, y, w, segHeight);
      } else {
        const segWidth = (w / numSegments) - segmentGap;
        const x = (holdIndex / numSegments) * w;
        ctx.fillRect(x, 0, segWidth, h);
      }

      ctx.restore();
    };

    drawPeakSegments();
    drawHoldIndicator();
  }, [peak, peakHold, orientation, height, width]);

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
