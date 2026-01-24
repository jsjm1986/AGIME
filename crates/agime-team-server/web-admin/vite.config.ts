import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { resolve } from 'path';

export default defineConfig({
  plugins: [react(), tailwindcss()],

  base: '/admin/',

  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },

  build: {
    target: ['es2020', 'edge89', 'firefox78', 'chrome80', 'safari14'],
    outDir: 'dist',
    emptyDirBeforeWrite: true,
  },

  server: {
    host: '127.0.0.1',
    port: 5180,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
    },
  },
});
