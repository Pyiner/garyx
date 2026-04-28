import React from 'react';
import ReactDOM from 'react-dom/client';

import { WebBotConsoleApp } from './App';
import '../renderer/src/styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <WebBotConsoleApp />
  </React.StrictMode>,
);
