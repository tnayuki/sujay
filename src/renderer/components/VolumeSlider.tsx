import React, { useRef, useState, useCallback, useEffect } from 'react';
import './VolumeSlider.css';

interface VolumeSliderProps {
  value: number; // 0-1
  onChange: (value: number) => void;
  label?: string;
}

const VolumeSlider: React.FC<VolumeSliderProps> = ({ value, onChange, label }) => {
  const [isDragging, setIsDragging] = useState(false);
  const sliderRef = useRef<HTMLDivElement>(null);

  const calculateValue = useCallback((clientY: number): number => {
    if (!sliderRef.current) return value;
    
    const rect = sliderRef.current.getBoundingClientRect();
    const y = clientY - rect.top;
    const height = rect.height;
    
    // Invert: top = 1.0, bottom = 0.0
    const newValue = 1 - (y / height);
    return Math.max(0, Math.min(1, newValue));
  }, [value]);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    const newValue = calculateValue(e.clientY);
    onChange(newValue);
  }, [calculateValue, onChange]);

  const handleMouseMove = useCallback((e: MouseEvent) => {
    if (!isDragging) return;
    const newValue = calculateValue(e.clientY);
    onChange(newValue);
  }, [isDragging, calculateValue, onChange]);

  const handleMouseUp = useCallback(() => {
    setIsDragging(false);
  }, []);

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

  // Calculate handle position (percentage from bottom)
  const handlePosition = value * 100;

  return (
    <div className="volume-slider-container">
      {label && <div className="volume-slider-label">{label}</div>}
      <div
        ref={sliderRef}
        className="volume-slider-track"
        onMouseDown={handleMouseDown}
      >
        <div className="volume-slider-fill" style={{ height: `${handlePosition}%` }} />
        <div
          className="volume-slider-handle"
          style={{ bottom: `calc(${handlePosition}% - 6px)` }}
        />
      </div>
      <div className="volume-slider-value">{Math.round(value * 100)}%</div>
    </div>
  );
};

export default VolumeSlider;
