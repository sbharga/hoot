import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    conditions: ['browser']
  },
  server: {
    port: 5173,
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', ws: true },
      '/media': { target: 'http://127.0.0.1:8080' }
    }
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts']
  }
});
