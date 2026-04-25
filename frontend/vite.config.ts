import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import path from 'path';
import { defineConfig, loadEnv } from 'vite';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, '.', '');
  return {
    base: mode === 'development' ? '/' : './',
    plugins: [react(), tailwindcss()],
    define: {
      'process.env.GEMINI_API_KEY': JSON.stringify(env.GEMINI_API_KEY),
    },
    resolve: {
      alias: {
        '@': path.resolve(__dirname, '.'),
      },
    },
    server: {
      // HMR is disabled in AI Studio via DISABLE_HMR env var.
      // Do not modify—file watching is disabled to prevent flickering during agent edits.
      hmr: process.env.DISABLE_HMR !== 'true',
      // Proxy API and SSR routes to the Rust backend during development.
      // Start the Rust server first: cargo run -- --cache-size 1GiB
      proxy: {
        '/api': 'http://localhost:8080',
        '/downloads': 'http://localhost:8080',
        '/robots.txt': 'http://localhost:8080',
        '/sitemap.xml': 'http://localhost:8080',
        '/healthz': 'http://localhost:8080',
      },
    },
  };
});
