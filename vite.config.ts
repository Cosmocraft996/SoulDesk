import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import vue from '@vitejs/plugin-vue';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [react(), vue()],
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/artifacts/**', '**/src-tauri/target/**'] } },
  clearScreen: false,
  build: {
    target: 'es2022',
    assetsInlineLimit: file => file.includes('notification-overlay') ? false : undefined,
    rollupOptions: {
      input: {
        app: fileURLToPath(new URL('./index.html', import.meta.url)),
        island: fileURLToPath(new URL('./island.html', import.meta.url)),
      },
    },
  },
});
