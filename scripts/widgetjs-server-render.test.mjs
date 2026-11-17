import test from 'node:test';import assert from 'node:assert/strict';import{createSSRApp}from'vue';import{renderToString}from'vue/server-renderer';import{createServer}from'vite';import{resolve}from'node:path';
test('服务器组件初始模板和缺失指标可实际渲染，不在预览中发起网络请求',async()=>{
  // 仅静态模板执行；不是浏览器、画布或桌面验收。
  const oldDocument=globalThis.document,oldCSS=globalThis.CSS;globalThis.document={documentElement:{style:{setProperty(){}}}};globalThis.CSS={supports:()=>true};
  const vite=await createServer({configFile:resolve('components/widgetjs/server-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,watch:null},appType:'custom',logLevel:'error'});
  try{const{default:App}=await vite.ssrLoadModule('/App.vue');const host={preview:true,request(){throw new Error('预览不应联网');},ready(){throw new Error('静态渲染不应启动视图');}};const settings={background:'#000000',color:'#ffffff',borderColor:'#333333',fontSize:16,radius:24};
    const empty=await renderToString(createSSRApp(App,{host,stored:{},settings}));assert.match(empty,/服务器监控/);assert.match(empty,/尚未添加/);
    const stored={version:1,servers:[{id:'one',name:'测试服务器',ip:'47.1.2.3',api:{url:'http://127.0.0.1:5549/api/monitor',token:'fixture',interval:10}}],master:{url:'',token:''}};const html=await renderToString(createSSRApp(App,{host,stored,settings}));assert.match(html,/测试服务器/);assert.match(html,/47\.\*\*\.\*\*\.\*\*/);assert.match(html,/尚未读取指标/);assert.ok(!html.includes('NaN'));assert.ok(!html.includes('undefined%'));
  }finally{await vite.close();if(oldDocument===undefined)delete globalThis.document;else globalThis.document=oldDocument;if(oldCSS===undefined)delete globalThis.CSS;else globalThis.CSS=oldCSS;}
});
test('实际Vue响应式表单可保存、同步和删除，不把代理对象直接提交到克隆接口',async()=>{
  const oldDocument=globalThis.document,oldCSS=globalThis.CSS;globalThis.document={documentElement:{style:{setProperty(){}}}};globalThis.CSS={supports:()=>true};
  const vite=await createServer({configFile:resolve('components/widgetjs/server-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,watch:null},appType:'custom',logLevel:'error'});let state;
  try{const{default:App}=await vite.ssrLoadModule('/App.vue'),writes=[],requests=[];const Subject={...App,setup(props,context){state=App.setup(props,context);return state;}};
    const host={preview:false,async request(method,args){requests.push([method,args]);if(method==='storage.set'){writes.push(structuredClone(args));return null;}if(method==='server.request')return args.kind==='pull'?{data:[],receivedAt:Date.now()}:args.kind==='push'?{accepted:true}:{data:{cpu:{cores:8,processors:1},osInfo:{distro:'Ubuntu'},mem:{total:1000}},receivedAt:Date.now()};throw new Error('未预期的方法');}};
    await renderToString(createSSRApp(Subject,{host,stored:{},settings:{background:'#000000',color:'#ffffff',borderColor:'#333333',fontSize:16,radius:24}}));
    for(const key of ['editDialog','syncDialog','deleteDialog'])state[key].value={showModal(){},close(){}};
    state.openEdit();Object.assign(state.draft.value,{name:'响应式服务器',ip:'47.1.2.3'});Object.assign(state.draft.value.api,{url:'http://127.0.0.1:5549/api/monitor',token:'fixture'});await state.saveServer();assert.equal(writes.length,1,state.message.value);assert.equal(writes[0].servers[0].name,'响应式服务器');
    state.openSync();Object.assign(state.sync.value,{url:'https://master.example/api/monitor',token:'fixture-master'});await state.syncServers('pull');assert.equal(writes.length,3,state.message.value);assert.equal(writes[2].master.token,'fixture-master');
    state.pendingDelete.value=state.draft.value.id=state.model.snapshot().servers[0].id;await state.removeServer();assert.equal(writes.at(-1).servers.length,0,state.message.value);assert.ok(requests.some(([method,args])=>method==='server.request'&&args.kind==='push'));
  }finally{state?.model.dispose();state?.reader.dispose();await vite.close();if(oldDocument===undefined)delete globalThis.document;else globalThis.document=oldDocument;if(oldCSS===undefined)delete globalThis.CSS;else globalThis.CSS=oldCSS;}
});
