import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import { ScreenSaverApp } from './components/ScreenSaverApp';
import type { ScreenSaverBootstrap } from './screensaver';
import { initializeSdk } from './sdk/runtime';
import './index.css';

await initializeSdk();
let screenSaver: ScreenSaverBootstrap | null = null;
try {
  const { invoke } = await import('@tauri-apps/api/core');
  screenSaver = await invoke<ScreenSaverBootstrap | null>('screen_saver_bootstrap');
} catch {
  // Plain browser mode has no native screen saver bootstrap.
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {screenSaver ? <ScreenSaverApp bootstrap={screenSaver} /> : <App />}
  </StrictMode>,
);
