/**
 * Vite configuration for Web build
 *
 * This config builds a standalone web version of AGIME that can be served
 * from the goose-server and accessed via the tunnel.
 *
 * Usage:
 *   npm run build:web
 *
 * Output:
 *   dist-web/
 */

import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { resolve } from 'path';
import { renameSync, existsSync } from 'fs';

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    // Plugin to rewrite root path to index-web.html in dev mode
    // and rename output to index.html for production
    {
      name: 'html-entry-rewrite',
      configureServer(server) {
        server.middlewares.use((req, _res, next) => {
          // Rewrite /web/ to index-web.html
          if (req.url === '/web/' || req.url === '/web') {
            req.url = '/index-web.html';
          }
          next();
        });
      },
      // Rename index-web.html to index.html after build
      closeBundle() {
        const srcPath = resolve(__dirname, 'dist-web/index-web.html');
        const destPath = resolve(__dirname, 'dist-web/index.html');
        if (existsSync(srcPath)) {
          renameSync(srcPath, destPath);
          console.log('Renamed index-web.html to index.html');
        }
      },
    },
  ],

  // Public assets directory for web build
  publicDir: 'public-web',

  // Base path for all assets - the web UI is served at /web/
  base: '/web/',

  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
    extensions: ['.ts', '.tsx', '.js', '.jsx', '.json'],
  },

  define: {
    // Mark this as a web build
    'import.meta.env.VITE_PLATFORM': JSON.stringify('web'),
    // Version info
    'import.meta.env.VITE_APP_VERSION': JSON.stringify(process.env.npm_package_version || 'web'),
    // Disable tunnel UI on web (it IS accessed via tunnel)
    'process.env.GOOSE_TUNNEL': JSON.stringify(false),
    'process.env.ALPHA': JSON.stringify(false),
  },

  build: {
    // Use ES2020 for broader browser compatibility (iOS Safari 14+, Chrome 80+)
    target: ['es2020', 'edge89', 'firefox78', 'chrome80', 'safari14'],
    outDir: 'dist-web',
    emptyDirOnBuild: true,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index-web.html'),
      },
      output: {
        // Consistent chunk naming for better caching
        entryFileNames: 'assets/[name]-[hash].js',
        chunkFileNames: 'assets/[name]-[hash].js',
        assetFileNames: 'assets/[name]-[hash].[ext]',
      },
    },
    // Rename index-web.html to index.html in output
    // This is handled by the postBuild hook below
    chunkSizeWarningLimit: 1000,
  },

  // Development server config (for local testing)
  server: {
    host: '127.0.0.1',
    port: 5174, // Different port from Electron dev server
    strictPort: true,
    // Proxy API calls to local goose-server during development
    proxy: {
      '/reply': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/sessions': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/agent': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/config': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/status': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/recipes': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/audio': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/tunnel': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
      '/schedule': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
    },
  },

  preview: {
    host: '127.0.0.1',
    port: 5175,
    strictPort: true,
  },
});
