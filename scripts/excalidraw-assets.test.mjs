import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve, posix } from 'node:path';
import { pathToFileURL } from 'node:url';
import { widgetFiles } from './build-widgetjs.mjs';
import { randomUUID } from 'node:crypto';
import { createRequire } from 'node:module';

test('画板标准包包含原234个字体、工作线程依赖与当前SDK，满足原包容量限制',()=>{
  const files=widgetFiles('excalidraw');
  assert.equal(Object.keys(files).filter(path=>path.startsWith('dist/fonts/')&&path.endsWith('.woff2')).length,234);
  assert.ok(Object.keys(files).length<=256);
  assert.ok(Object.values(files).reduce((size,data)=>size+data.length,0)<=50*1024*1024);
  assert.deepEqual(files['dist/host.js'],new Uint8Array(readFileSync('sdk/host.js')));
  assert.deepEqual(JSON.parse(new TextDecoder().decode(files['manifest.json'])),JSON.parse(readFileSync('components/widgetjs/excalidraw/manifest.json','utf8')));
  const js=new TextDecoder().decode(files['dist/app.js']);
  assert.doesNotMatch(js,/from\s*["'](?:browser-fs-access|@widget-js\/)/);
  assert.ok(js.includes('./worker/subset-worker.chunk.js'));
  for(const [path,bytes]of Object.entries(files).filter(([path])=>path.startsWith('dist/worker/')&&path.endsWith('.js'))){
    const source=new TextDecoder().decode(bytes);
    for(const match of source.matchAll(/(?:from|import)\s*["']\.\/([^"']+\.js)["']/g))assert.ok(files[posix.join(posix.dirname(path),match[1])],path+'缺少工作线程模块'+match[1]);
  }
});

test('随包字体子集代码能实际生成SVG所用的WOFF2字形子集',async()=>{
  const files=widgetFiles('excalidraw'),root=resolve('artifacts','excalidraw-font-test-'+randomUUID());
  mkdirSync(root,{recursive:true});writeFileSync(join(root,'package.json'),'{"type":"module"}');
  for(const [path,data]of Object.entries(files).filter(([path])=>path.startsWith('dist/worker/'))){writeFileSync(join(root,path.split('/').at(-1)),data);}
  const module=await import(pathToFileURL(join(root,'chunk-EIO257PC.js')).href);
  const font=files['dist/fonts/Virgil/Virgil-Regular.woff2'];
  assert.ok(font);
  const buffer=font.buffer.slice(font.byteOffset,font.byteOffset+font.byteLength);
  // 发布库的Node分支期望CommonJS环境；补环境变量后仍执行随包原始字节。
  const previousDirectory=globalThis.__dirname,previousRequire=globalThis.require;
  globalThis.__dirname=root;globalThis.require=createRequire(import.meta.url);
  try {
    const result=new Uint8Array(await module.c(buffer,[65,66,67,49,50,51]));
    assert.equal(new TextDecoder().decode(result.subarray(0,4)),'wOF2');
    assert.ok(result.length>100 && result.length<font.length);
  } finally {
    if(previousDirectory===undefined)delete globalThis.__dirname;else globalThis.__dirname=previousDirectory;
    if(previousRequire===undefined)delete globalThis.require;else globalThis.require=previousRequire;
  }
});
