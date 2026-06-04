import React from 'react';
import ReactDOM from 'react-dom/client';

import { App } from './App';
import {
  installDesktopApiPerformanceMonitor,
  startRendererPerformanceMonitor,
} from './perf-metrics';
import './styles.css';

startRendererPerformanceMonitor();
installDesktopApiPerformanceMonitor();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
