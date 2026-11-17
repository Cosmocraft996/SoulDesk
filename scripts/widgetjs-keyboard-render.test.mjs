import test from 'node:test';
import assert from 'node:assert/strict';
import {createSSRApp} from 'vue';
import {renderToString} from 'vue/server-renderer';
import {createServer} from 'vite';
import {resolve} from 'node:path';

test('键盘演示原模板显示修饰键与Caps状态，隐藏快照不泄漏旧键', async () => {
  const vite = await createServer({configFile: resolve('components/widgetjs/key-stroke-source/vite.config.mjs'), server:{middlewareMode:true,hmr:false,ws:false,watch:null},appType:'custom',logLevel:'silent',optimizeDeps:{noDiscovery:true,include:[]}});
  try {
    const {default:Overlay} = await vite.ssrLoadModule('/Overlay.vue');
    const snapshot={visible:true,modifierKeys:[{keyCode:162,name:'Ctrl',isKeyUp:false}],normalKey:{keyCode:67,name:'C',isKeyUp:true},capsLock:{keyCode:20,isKeyUp:false},isCapsLockOn:true};
    const html=await renderToString(createSSRApp(Overlay,{snapshot}));
    assert.match(html,/keyboard-inner/);assert.match(html,/Ctrl/);assert.match(html,/class="(?=[^"]*caps-lock)(?=[^"]*active)(?=[^"]*on)[^"]*"/);assert.match(html,/>C</);
    const hidden=await renderToString(createSSRApp(Overlay,{snapshot:{...snapshot,visible:false,modifierKeys:[],normalKey:null,capsLock:null}}));
    assert.ok(!hidden.includes('Ctrl'));assert.ok(!hidden.includes('is-visible'));
  } finally {await vite.close();}
});

test('控制卡默认关闭，保存失败保留开关，重试展示实际服务错误',async()=>{
  const vite=await createServer({configFile:resolve('components/widgetjs/key-stroke-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,ws:false,watch:null},appType:'custom',logLevel:'silent',optimizeDeps:{noDiscovery:true,include:[]}});
  try{
    const {default:App}=await vite.ssrLoadModule('/App.vue');let state;let fail=true;const calls=[];
    const host={preview:false,async request(method,args){calls.push([method,args]);if(fail)throw Error('保存失败');return {enabled:args.enabled,listening:false,suspended:false,lastError:'叠加层初始化失败'};}};
    const Subject={...App,setup(props,ctx){state=App.setup(props,ctx);return state;}};
    const html=await renderToString(createSSRApp(Subject,{host}));assert.match(html,/启用键盘演示/);assert.equal(state.status.value.enabled,false);
    await state.configure(true);assert.equal(state.status.value.enabled,false);assert.match(state.error.value,/保存失败/);
    fail=false;await state.configure(true);assert.equal(state.status.value.enabled,true);assert.equal(state.status.value.listening,false);assert.match(state.status.value.lastError,/初始化失败/);assert.equal(calls.length,2);
  }finally{await vite.close();}
});

test('启用完成后晚到的旧状态轮询不能把控制卡改回停用',async()=>{
  const vite=await createServer({configFile:resolve('components/widgetjs/key-stroke-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,ws:false,watch:null},appType:'custom',logLevel:'silent',optimizeDeps:{noDiscovery:true,include:[]}});
  try{
    const{default:App}=await vite.ssrLoadModule('/App.vue');let state,finish;
    const host={preview:false,request(method,args){if(method==='keyboard.status')return new Promise(resolve=>finish=resolve);return Promise.resolve({enabled:args.enabled,listening:false,suspended:false,lastError:null});}};
    const Subject={...App,setup(props,ctx){state=App.setup(props,ctx);return state;}};await renderToString(createSSRApp(Subject,{host}));
    const pending=state.refresh();await state.configure(true);finish({enabled:false,listening:false,suspended:false,lastError:null});await pending;assert.equal(state.status.value.enabled,true);
  }finally{await vite.close();}
});
