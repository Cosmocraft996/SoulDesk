import{defineConfig}from'vite';import vue from'@vitejs/plugin-vue';import{fileURLToPath}from'node:url';
import{readFileSync,existsSync,readdirSync}from'node:fs';import{dirname,join}from'node:path';
import { entryPlugin } from '../../../scripts/widgetjs-entry.mjs';
function bundledLicenses(){return{name:'固定AI依赖来源',generateBundle(options,bundle){const packages=new Map();for(const chunk of Object.values(bundle)){if(chunk.type!=='chunk')continue;for(const id of Object.keys(chunk.modules)){if(!id.includes('node_modules')||id.includes('\0'))continue;let directory=dirname(id.split('?')[0]);while(dirname(directory)!==directory){const metadata=join(directory,'package.json');if(existsSync(metadata)){let pkg;try{pkg=JSON.parse(readFileSync(metadata,'utf8'));}catch{break;}if(pkg.name&&pkg.version){const key=pkg.name+'@'+pkg.version;if(!packages.has(key)){const licenses=readdirSync(directory,{withFileTypes:true}).filter(file=>file.isFile()&&/^(licen[sc]e|copying|notice)(?:\.|$)/i.test(file.name)).map(file=>file.name+'\n'+readFileSync(join(directory,file.name),'utf8'));packages.set(key,[key,'许可：'+(typeof pkg.license==='string'?pkg.license:JSON.stringify(pkg.license??'未声明')), ...licenses].join('\n'));}break;}}directory=dirname(directory);}}}this.emitFile({type:'asset',fileName:'THIRD_PARTY_LICENSES.txt',source:[...packages.entries()].sort(([a],[b])=>a.localeCompare(b)).map(([,text])=>text).join('\n\n----------------\n\n')+'\n'});}};}
export { bundledLicenses };
export default defineConfig(({ mode }) => {
  const root = fileURLToPath(new URL('.', import.meta.url));
  const variant = { 'ai-mindmap': 'mindmap', 'ai-mermaid': 'mermaid', 'ai-translate': 'translate', 'ai-assistant': 'assistant' }[mode] ?? 'translate';
  const source = [
    "import { connectHost } from '../../../sdk/host.js';",
    variant === 'assistant'
      ? "import View from './Assistant.vue'; import { mountWidget } from './main.js'; await mountWidget(View, { host: await connectHost() });"
      : `import View from './views/${{ mindmap: 'Mindmap', mermaid: 'Mermaid', translate: 'Translate' }[variant]}.vue'; import { mountGenerator } from './main.js'; await mountGenerator(await connectHost(), '${variant}', View);`,
  ].join('\n');
  return { root, base: './', publicDir: false, plugins: [vue(), bundledLicenses(), entryPlugin(root, source)], build: {
    target: 'es2022', outDir: '../ai-bundle', emptyOutDir: false, assetsInlineLimit: 0, cssCodeSplit: false, modulePreload: false,
    rollupOptions: { input: 'virtual:souldesk-widget-entry', output: { entryFileNames: 'app.js', chunkFileNames: '[name]-[hash].js', assetFileNames: asset => asset.names?.some(name => name.endsWith('.css')) ? 'style.css' : '[name][extname]' } },
  } };
});
