import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import './app.css';
import './shell.css';
import './components/controls.css';
import App from './App';
import { previewRequested } from './preview/gate';

const render = () => createRoot(document.getElementById('root')!).render(
  <StrictMode><App /></StrictMode>,
);
// import.meta.env.DEV is false in production builds, so the preview module is never bundled there.
if (import.meta.env.DEV && previewRequested(import.meta.env.DEV, location.search)) {
  void import('./preview/backend').then(preview => { preview.installPreview(); render(); });
} else render();
