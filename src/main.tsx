import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';

const rootEl = document.getElementById('root');
if (!rootEl) {
  throw new Error('missing #root element');
}
createRoot(rootEl).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// PWA service worker: registered only when served over http(s), never inside
// the Tauri webview (Tauri serves via its own protocol).
if ('serviceWorker' in navigator && window.location.protocol.startsWith('http')) {
  window.addEventListener('load', () => {
    void navigator.serviceWorker.register('./sw.js').catch(() => undefined);
  });
}
