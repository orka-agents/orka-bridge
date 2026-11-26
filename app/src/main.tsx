import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import './app.css';
import './shell.css';
import './components/controls.css';
import App from './App';
import { applyTheme, readTheme } from './chrome/theme';
import { previewRequested } from './preview/gate';

applyTheme(readTheme());
const render = () => createRoot(document.getElementById('root')!).render(
  <StrictMode><App /></StrictMode>,
);
// import.meta.env.DEV is false in production builds, so the preview module is never bundled there.
if (import.meta.env.DEV && previewRequested(import.meta.env.DEV, location.search)) {
  // `&gallery` shows the shared components instead of the app; it is part of the dev-only preview.
  if (new URLSearchParams(location.search).has('gallery')) {
    void import('./components/Gallery').then(({ default: Gallery }) => createRoot(document.getElementById('root')!).render(<StrictMode><Gallery /></StrictMode>));
  } else void import('./preview/backend').then(preview => { preview.installPreview(); render(); });
} else render();
