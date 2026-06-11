import { resolve } from 'node:path';

import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Conversation-state storybook: a standalone web page that mounts the real
// renderer transcript components against scripted contract states. Run with
// `npm run storybook`; it never ships with the app build.
export default defineConfig({
  root: resolve(__dirname, 'src/renderer'),
  build: {
    outDir: resolve(__dirname, 'out/storybook'),
    emptyOutDir: true,
    rollupOptions: {
      input: resolve(__dirname, 'src/renderer/storybook.html'),
    },
  },
  resolve: {
    alias: {
      '@renderer': resolve(__dirname, 'src/renderer/src'),
      '@shared': resolve(__dirname, 'src/shared'),
      '@': resolve(__dirname, 'src/renderer/src'),
    },
  },
  plugins: [tailwindcss(), react()],
});
