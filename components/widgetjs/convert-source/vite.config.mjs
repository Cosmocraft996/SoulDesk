import {defineConfig} from 'vite';
import {fileURLToPath} from 'node:url';
export default defineConfig({root:fileURLToPath(new URL('.',import.meta.url)),base:'./',publicDir:false,
  build:{target:'es2022',outDir:'../convert-bundle',emptyOutDir:false,assetsInlineLimit:0,cssCodeSplit:false,modulePreload:false,
    rollupOptions:{output:{inlineDynamicImports:true,entryFileNames:'app.js',assetFileNames:asset=>asset.names?.some(name=>name.endsWith('.css'))?'style.css':'assets/[name]-[hash][extname]'}},
  },
});
