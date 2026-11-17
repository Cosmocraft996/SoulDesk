import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';
import tailwind from '@tailwindcss/vite';
import {fileURLToPath} from 'node:url';
export default defineConfig({root:fileURLToPath(new URL('.',import.meta.url)),base:'./',publicDir:false,plugins:[react(),tailwind()],resolve:{alias:{'@':fileURLToPath(new URL('./ui',import.meta.url))}},build:{target:'es2022',outDir:'../mindmap-bundle',emptyOutDir:false,assetsInlineLimit:0,cssCodeSplit:false,modulePreload:false,rollupOptions:{output:{inlineDynamicImports:true,entryFileNames:'app.js',assetFileNames:asset=>asset.names?.some(name=>name.endsWith('.css'))?'style.css':'[name]-[hash][extname]'}}}});
