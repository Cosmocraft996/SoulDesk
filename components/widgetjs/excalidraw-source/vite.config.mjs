import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
export default defineConfig({
  root:fileURLToPath(new URL('.',import.meta.url)),base:'./',plugins:[react()],publicDir:false,
  resolve:{alias:[
    {find:'browser-fs-access',replacement:fileURLToPath(new URL('./file-access.js',import.meta.url))},
    {find:/^\.\/subset-worker\.chunk\.js$/,replacement:fileURLToPath(new URL('./worker-location.js',import.meta.url))},
  ]},
  build:{target:'es2022',outDir:'../excalidraw-bundle',emptyOutDir:false,assetsInlineLimit:0,cssCodeSplit:false,modulePreload:false,
    rollupOptions:{output:{inlineDynamicImports:true,entryFileNames:'app.js',assetFileNames:asset=>asset.names?.some(name=>name.endsWith('.css'))?'style.css':'assets/[name]-[hash][extname]'}},
  },
});
