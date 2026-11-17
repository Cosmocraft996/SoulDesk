// 运行原Vue派生控件与真实状态客户端；只有离线DOM和假宿主，无浏览器或系统输入。
import test from'node:test';import assert from'node:assert/strict';import{readFileSync}from'node:fs';import{resolve,dirname}from'node:path';import{pathToFileURL}from'node:url';import{JSDOM}from'jsdom';
import{createAssistantClient,defaultAssistantConfig}from'../components/widgetjs/ai-source/assistant-client.js';
const dom=new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>',{url:'http://fixture.local/'});for(const key of['window','document','Element','HTMLElement','SVGElement','Node','navigator','MutationObserver','MouseEvent','FocusEvent'])Object.defineProperty(globalThis,key,{value:key==='window'?dom.window:key==='document'?dom.window.document:dom.window[key],configurable:true});globalThis.getComputedStyle=dom.window.getComputedStyle;globalThis.requestAnimationFrame=f=>setTimeout(f,0);globalThis.cancelAnimationFrame=clearTimeout;
const{createApp,h,provide,ref,computed,nextTick}=await import('vue'),{parse,compileScript,compileTemplate}=await import('vue/compiler-sfc'),ElementPlus=(await import('element-plus')).default,cache=new Map();
function moduleUrl(path){path=resolve(path);if(cache.has(path))return cache.get(path);const d=parse(readFileSync(path,'utf8')).descriptor;let code=d.script||d.scriptSetup?compileScript(d,{id:'assistant_'+cache.size,inlineTemplate:true}).content:compileTemplate({source:d.template.content,filename:path,id:'assistant_'+cache.size}).code.replace('export function render','function render')+'\nexport default{render};';code=code.replace(/from\s*(['"])([^'"]+)\1/g,(all,q,name)=>'from '+JSON.stringify(name.endsWith('.vue')?moduleUrl(resolve(dirname(path),name)):name.startsWith('.')?pathToFileURL(resolve(dirname(path),name)).href:import.meta.resolve(name)));const url='data:text/javascript;base64,'+Buffer.from(code).toString('base64');cache.set(path,url);return url;}
async function component(name){return(await import(moduleUrl('components/widgetjs/ai-source/assistant/'+name+'.vue'))).default;}
const flush=async()=>{await new Promise(r=>setTimeout(r,15));await nextTick();};
function fixture(){
 const state=ref(),server={revision:0,config:{...defaultAssistantConfig(),tipProgress:3},surface:{presentation:'fixture',serial:1,phase:'ready',nativeSettled:true,overlays:[]}},calls=[];
 let configGate,failSave=false;
 const host={request:async(method,args)=>{calls.push([method,args?JSON.parse(JSON.stringify(args)):null]);if(method==='assistant.configure'){if(configGate)await configGate;if(failSave)throw failSave instanceof Error?failSave:Error('STORAGE_ERROR: 测试拒绝保存');if(args.expectedRevision!==server.revision)throw Error('REVISION_CONFLICT: 设置已变化');server.revision++;server.config=JSON.parse(JSON.stringify(args.config));}else if(method==='assistant.overlay'){server.surface.serial++;server.surface.overlays=args.visible?[args.id]:[];}return JSON.parse(JSON.stringify(server));}};
 const client=createAssistantClient(host,{shell:true,changed:value=>state.value=value});state.value={...client.state};
 const services=['chatgpt','grok','gemini','deepseek','kimi','qwen','dou_bao','yuan_bao'].map(value=>({value,name:value,logo:'data:image/png;base64,'}));
 const AiUtils={usAiServices:services.slice(0,3),chinaAiServices:services.slice(3),get:value=>services.find(p=>p.value===value)??services[0]};
 const ui={state,client,host,AiUtils,t:k=>k,locked:computed(()=>state.value.busy||state.value.uncertain),error:computed(()=>state.value.error),settings:async()=>{},platform:(i,v)=>{const p=[...state.value.config.platforms];p[i]=v;return client.quick({platforms:p});}};
 return{ui,server,calls,gate:p=>configGate=p,fail:v=>failSave=v};
}
async function mount(Component,ui,props=()=>({})){const app=createApp({setup(){provide('assistant-ui',ui);return()=>h(Component,props());}}).use(ElementPlus);app.mount('#app');await flush();return app;}
test.after(()=>dom.window.close());
test('选择平台先隐藏网页，等待保存完成后关闭菜单并恢复网页',async()=>{
 const f=fixture();await f.ui.client.load();const Select=await component('Select');const app=await mount(Select,f.ui,()=>({index:0,modelValue:f.ui.state.value.config.platforms[0],'onUpdate:modelValue':v=>f.ui.platform(0,v)}));
 try{
  document.querySelector('.select-trigger').click();await flush();await flush();assert.equal(f.calls.find(c=>c[0]==='assistant.overlay')[1].visible,true);
  let resolveSave;f.gate(new Promise(r=>resolveSave=r));const option=[...document.querySelectorAll('[role="option"]')].find(e=>e.textContent.includes('deepseek'));assert.ok(option,'展开原ElSelect平台选项');option.click();await flush();
  assert.equal(f.calls.filter(c=>c[0]==='assistant.configure').length,1);assert.equal(f.calls.filter(c=>c[0]==='assistant.overlay'&&c[1].visible===false).length,0);
  resolveSave();await flush();await flush();assert.equal(f.ui.state.value.config.platforms[0],'deepseek');assert.equal(f.calls.filter(c=>c[0]==='assistant.overlay'&&c[1].visible===false).length,1);assert.ok(document.querySelector('.select-trigger'));
 }finally{app.unmount();f.ui.client.dispose();}
});
test('原设置保存失败保留输入，后台轮询不替换草稿或保存基线',async()=>{
 const f=fixture();await f.ui.client.load();const Settings=await component('Settings');const app=await mount(Settings,f.ui);
 try{
  const input=document.querySelector('input[placeholder="Super+Alt+C"]');input.value='Super+Alt+K';input.dispatchEvent(new dom.window.Event('input',{bubbles:true}));
  f.fail(true);document.querySelector('form').dispatchEvent(new dom.window.Event('submit',{bubbles:true,cancelable:true}));await flush();assert.equal(input.value,'Super+Alt+K');assert.match(document.querySelector('[role="alert"]').textContent,/STORAGE_ERROR/);
  f.server.revision=4;f.server.config.shortcut='Super+Alt+J';f.fail(false);await f.ui.client.refresh();await flush();assert.equal(input.value,'Super+Alt+K');
  document.querySelector('form').dispatchEvent(new dom.window.Event('submit',{bubbles:true,cancelable:true}));await flush();const saves=f.calls.filter(c=>c[0]==='assistant.configure');assert.deepEqual(saves.map(c=>c[1].expectedRevision),[0,0]);assert.equal(saves.at(-1)[1].config.shortcut,'Super+Alt+K');assert.deepEqual(saves.at(-1)[1].config.proxy,{mode:'system'});
 }finally{app.unmount();f.ui.client.dispose();}
});
test('平台保存超时保留隐藏状态，重试原UUID成功后恢复选择器与网页',async()=>{
 const f=fixture();await f.ui.client.load();const Select=await component('Select');const app=await mount(Select,f.ui,()=>({index:0,modelValue:f.ui.state.value.config.platforms[0]}));
 try{
  document.querySelector('.select-trigger').click();await flush();await flush();f.fail(Error('HOST_TIMEOUT: 保存结果未确定'));
  [...document.querySelectorAll('[role="option"]')].find(e=>e.textContent.includes('deepseek')).click();await flush();
  assert.equal(f.ui.state.value.uncertain,true);assert.deepEqual(f.server.surface.overlays,['select0']);
  f.fail(false);assert.equal(await f.ui.client.retrySave(),true);await flush();await flush();
  const saves=f.calls.filter(c=>c[0]==='assistant.configure');assert.equal(saves.length,2);assert.deepEqual(saves[0][1],saves[1][1]);
  assert.deepEqual(f.server.surface.overlays,[]);assert.ok(document.querySelector('.select-trigger'));
 }finally{app.unmount();f.ui.client.dispose();}
});
