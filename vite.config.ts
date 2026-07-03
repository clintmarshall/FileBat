import { defineConfig } from 'vite';
import Inspect from 'vite-plugin-inspect';

export default defineConfig({
  plugins: [Inspect()],
  root: './src',
  build: {
    outDir: '../src-tauri/frontend-dist',
    target: 'ES2022',
    rollupOptions: {
      // Tauri API is provided by the runtime, not bundled
      external: ['@tauri-apps/api', '@tauri-apps/api/core'],
    },
  },
  optimizeDeps: {
    // Don't pre-bundle Tauri APIs — they're injected by the webview
    exclude: ['@tauri-apps/api'],
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    include: ['**/*.test.ts'],
  },
});
