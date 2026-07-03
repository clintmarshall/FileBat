import { defineConfig } from 'vitest/config';
import Inspect from 'vite-plugin-inspect';

export default defineConfig({
  clearScreen: false, 

  server: {
    port: 1420,
    strictPort: true,
  },

  plugins: [
    // ✅ Fix: Only inject the dashboard if running our separate inspect script
    ...(process.argv.includes('1421') ? [Inspect()] : [])
  ],

  test: {
    environment: 'jsdom',
    globals: false,
    setupFiles: ['./src/test/setup.ts'],
  },
});
