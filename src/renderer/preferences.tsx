import React from 'react';
import { createRoot } from 'react-dom/client';

import '../index.css';
import './App.css';
import Preferences from './components/Preferences';

const container = document.body;
const root = createRoot(container);

const handleClose = () => {
  window.close();
};

root.render(<Preferences onClose={handleClose} />);
