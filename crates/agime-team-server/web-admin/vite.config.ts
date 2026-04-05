import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { extname, resolve } from 'path';

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
    rollupOptions: {
      output: {
        entryFileNames: 'assets/entry-[name]-[hash].js',
        chunkFileNames: 'assets/chunk-[name]-[hash].js',
        assetFileNames: (assetInfo) => {
          const extension = extname(assetInfo.name || '').toLowerCase();
          if (extension === '.css') {
            return 'assets/style-[name]-[hash][extname]';
          }
          if (['.woff', '.woff2', '.ttf', '.otf', '.eot'].includes(extension)) {
            return 'assets/font-[name]-[hash][extname]';
          }
          return 'assets/asset-[name]-[hash][extname]';
        },
      },
    },
  },

  server: {
    host: '127.0.0.1',
    port: 5180,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
        // Ensure cookies are properly forwarded
        cookieDomainRewrite: '',
        secure: false,
      },
    },
  },
});
