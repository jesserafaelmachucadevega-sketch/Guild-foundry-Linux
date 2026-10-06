import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Renderer (UI) build. Output goes to dist/ so Electron can load it from disk,
// and the same bundle is served as the PWA fallback.
export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});
