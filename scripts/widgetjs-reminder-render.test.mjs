import test from 'node:test';import assert from 'node:assert/strict';import{existsSync}from'node:fs';import{resolve}from'node:path';import{createSSRApp}from'vue';import{renderToString}from'vue/server-renderer';import{createServer}from'vite';
async function withVue(run){assert.ok(existsSync('components/widgetjs/reminder-source/vite.config.mjs'),'原提醒界面尚未适配');const vite=await createServer({configFile:resolve('components/widgetjs/reminder-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,ws:false,watch:null},appType:'custom',logLevel:'silent',optimizeDeps:{noDiscovery:true,include:[]}});try{await run(vite);}finally{await vite.close();}}
test('原灵动通知模板保留八种内容、双按钮、原来电头像和倒计时进度',()=>withVue(async vite=>{
  const{default:Island}=await vite.ssrLoadModule('/Island.vue');const host={preview:true,request(){assert.fail('预览不能调用后台');}};
  for(const kind of ['info','success','error','warning','reminder','advance-countdown','countdown','call']){
    const html=await renderToString(createSSRApp(Island,{host,snapshot:{sampledAt:1000,displayed:false,notification:{id:'fixed',kind,title:'久坐提醒',message:'消息内容',targetAt:61000,createdAt:1000,preset:'tea',mute:true}}}));
    assert.match(html,/class="island/);if(kind==='reminder'){assert.match(html,/知道了/);assert.match(html,/休息一下/);}else if(kind==='call'){assert.match(html,/elvis\.png/);assert.match(html,/接听/);assert.match(html,/挂断/);assert.ok(!html.includes('<audio'));}else if(kind==='countdown'){assert.match(html,/01:00:00/);assert.match(html,/progress-bar/);}else if(kind==='advance-countdown')assert.match(html,/1 min/);else assert.match(html,/消息内容/);
  }
}));
test('三个控制卡默认关闭，来电保留四事项、工作日和第一时刻',()=>withVue(async vite=>{
  const{default:Control}=await vite.ssrLoadModule('/Control.vue');const host={preview:true,request(){assert.fail('预览不读写后台');}};
  for(const variant of ['island','sit','phone']){const html=await renderToString(createSSRApp(Control,{host,variant}));assert.match(html,/启用/);assert.ok(!/class="enable"[^>]*>\s*<input[^>]* checked/.test(html));assert.ok(!/aria-label="启用[^"]*"[^>]* checked/.test(html));if(variant==='sit')for(const label of ['久坐时长（分钟）','休息时长（分钟）','检测阈值（分钟）'])assert.ok(html.includes(label));if(variant==='phone')for(const label of ['下班提醒','饮茶提醒','吃药提醒','学习提醒','18:00','15:00','工作日','试听'])assert.ok(html.includes(label));}
}));
test('全屏休息原数字切换模板保留Esc退出，关闭失败可重试',()=>withVue(async vite=>{
  const{default:Break}=await vite.ssrLoadModule('/Break.vue');let state,calls=0;const host={async request(method,args){calls++;assert.equal(method,'sit.break.close');assert.deepEqual(args,{id:'rest'});if(calls===1)throw Error('暂时失败');}};
  const Subject={...Break,setup(p,c){state=Break.setup(p,c);return state;}};
  const html=await renderToString(createSSRApp(Subject,{host,snapshot:{id:'rest',remainingSeconds:65,sampledAt:1000}}));assert.match(html,/按Esc退出/);assert.match(html,/text-switcher/);await state.close();assert.match(state.error.value,/暂时失败/);await state.close();assert.equal(calls,2);
}));
test('来电挂断保存失败后保留事件，恢复后仍能再次接听',()=>withVue(async vite=>{
  const{default:Phone}=await vite.ssrLoadModule('/parts/Phone.vue');let state,plays=0,hangupError;
  const host={preview:false,async request(method,args){if(args.action==='hangup')throw Error('database is full');}};
  const Subject={...Phone,setup(p,c){state=Phone.setup(p,c);return state;}};
  await renderToString(createSSRApp(Subject,{host,displayed:true,notification:{id:'call',preset:'tea',mute:false,title:'x',message:'x'},onAction:action=>{void host.request('island.action',{id:'call',action}).catch(e=>{hangupError=e.message;});}}));
  state.audio.value={currentTime:0,pause(){},async play(){plays++;}};state.hangup.value={pause(){},async play(){}};
  await state.answerClick();state.hangupClick();await Promise.resolve();assert.equal(hangupError,'database is full');await state.answerClick();assert.equal(plays,2);
}));
