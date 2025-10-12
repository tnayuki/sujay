import React, { useState } from 'react';
import type { GeneratingTrack } from '../../types';
import './Generate.css';

interface GenerateProps {
  generatingQueue: GeneratingTrack[];
  onGenerate: (prompt: string) => void;
}

const Generate: React.FC<GenerateProps> = ({ generatingQueue, onGenerate }) => {
  const [prompt, setPrompt] = useState('');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (prompt.trim() && generatingQueue.length === 0) {
      onGenerate(prompt.trim());
      setPrompt('');
    }
  };

  const isGenerating = generatingQueue.length > 0;

  return (
    <div className="generate">
      {isGenerating && generatingQueue[0] ? (
        <div className="generating-status">
          <div className="generating-prompt">
            <strong>Generating:</strong> {generatingQueue[0].prompt}
          </div>
          <div className="generating-progress">
            Status: {generatingQueue[0].status}
          </div>
        </div>
      ) : (
        <form onSubmit={handleSubmit} className="generate-form">
          <input
            type="text"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="Describe the music you want to generate..."
            className="generate-input"
            disabled={isGenerating}
          />
          <button
            type="submit"
            className="generate-button"
            disabled={isGenerating || !prompt.trim()}
          >
            Generate
          </button>
        </form>
      )}
    </div>
  );
};

export default Generate;
