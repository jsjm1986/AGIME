import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';
import { resolve } from 'path';

// https://vitejs.dev/config
export default defineConfig({
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
    extensions: ['.ts', '.tsx', '.js', '.jsx', '.json'],
  },
  define: {
    // This replaces process.env.ALPHA with a literal at build time
    'process.env.ALPHA': JSON.stringify(process.env.ALPHA === 'true'),
    'process.env.GOOSE_TUNNEL': JSON.stringify(process.env.GOOSE_TUNNEL !== 'no' && process.env.GOOSE_TUNNEL !== 'none'),
  },

  plugins: [tailwindcss()],

  build: {
    target: 'esnext'
  },

  // Fix Windows Vite 7.x binding issue
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    // Force HTTP/1.1 for better Windows compatibility
    middlewareMode: false,
    // Proxy API calls to local agimed server
    proxy: {
      '/api/team': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
    },
  },
});
