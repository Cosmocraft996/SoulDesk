import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import tailwind from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
export default defineConfig({
  root:fileURLToPath(new URL('.',import.meta.url)),base:'./',publicDir:false,
  plugins:[vue(),tailwind()],resolve:{alias:{'@':fileURLToPath(new URL('.',import.meta.url))}},
  build:{target:'es2022',outDir:'../weather-bundle',emptyOutDir:false,assetsInlineLimit:Number.MAX_SAFE_INTEGER,cssCodeSplit:false,modulePreload:false,rollupOptions:{output:{inlineDynamicImports:true,entryFileNames:'app.js',assetFileNames:'style.css'}}},
});
