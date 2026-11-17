import { readFileSync, writeFileSync, readdirSync, mkdirSync, copyFileSync, renameSync } from 'node:fs';
import { join, dirname, resolve, relative } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { zipSync } from 'fflate';
const upstream = 'node_modules/@excalidraw/excalidraw/dist/prod';
const target = 'components/widgetjs/excalidraw-bundle';
function copyTree(source,destination) {
  mkdirSync(destination,{recursive:true});
  for (const entry of readdirSync(source,{withFileTypes:true})) {
    if(entry.isDirectory())copyTree(join(source,entry.name),join(destination,entry.name));
    else copyFileSync(join(source,entry.name),join(destination,entry.name));
  }
}
export function finishExcalidrawBundle() {
  if(JSON.parse(readFileSync('node_modules/@excalidraw/excalidraw/package.json','utf8')).version!=='0.18.0')throw new Error('画板库版本与原锁文件不符');
  copyTree(join(upstream,'fonts'),join(target,'fonts'));
  const copied = new Set();
  function worker(file) {
    if(copied.has(file))return;copied.add(file);
    const source=join(upstream,file),text=readFileSync(source,'utf8'),out=join(target,'worker',file);
    if(relative(resolve(upstream),resolve(source)).startsWith('..'))throw new Error('工作线程依赖越界');
    mkdirSync(dirname(out),{recursive:true});writeFileSync(out,text);
    for(const match of text.matchAll(/(?:from|import)\s*["']\.\/([^"']+\.js)["']/g))worker(match[1]);
  }
  worker('subset-worker.chunk.js');
  const files={};
  function add(source,prefix) {
    for(const entry of readdirSync(source,{withFileTypes:true})) {
      if(entry.isDirectory())add(join(source,entry.name),prefix+entry.name+'/');
      else if(!entry.name.startsWith('preset.zip') && entry.name!=='index.html') files[prefix+entry.name]=new Uint8Array(readFileSync(join(source,entry.name)));
    }
  }
  add(target,'dist/');
  for(const name of ['index.html','assets.js','SOURCE.txt','LICENSE.txt'])files['dist/'+name]=new Uint8Array(readFileSync('components/widgetjs/excalidraw/'+name));
  files['manifest.json']=new Uint8Array(readFileSync('components/widgetjs/excalidraw/manifest.json'));
  files['dist/host.js']=new Uint8Array(readFileSync('sdk/host.js'));
  const zip=zipSync(files,{level:9,mtime:new Date('2026-11-14T00:00:00Z')});
  writeFileSync(join(target,'preset.zip.pending'),zip);
  renameSync(join(target,'preset.zip.pending'),join(target,'preset.zip'));
  console.log(JSON.stringify({package:'excalidraw',files:Object.keys(files).length,expandedBytes:Object.values(files).reduce((n,b)=>n+b.length,0),zipBytes:zip.length}));
}
export function buildExcalidraw() {
  const built=spawnSync(process.execPath,['node_modules/vite/bin/vite.js','build','--config','components/widgetjs/excalidraw-source/vite.config.mjs'],{stdio:'inherit'});
  if(built.status!==0)throw new Error('画板资源构建失败');
  finishExcalidrawBundle();
}
if(process.argv[1] && import.meta.url===pathToFileURL(resolve(process.argv[1])).href)buildExcalidraw();
