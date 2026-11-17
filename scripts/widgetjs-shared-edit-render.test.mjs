import test from'node:test';import assert from'node:assert/strict';
import{createSSRApp}from'vue';import{renderToString}from'vue/server-renderer';import{createServer}from'vite';import{resolve}from'node:path';
const id='1f282c1b-9bc8-461a-87cc-98de36e334a6';
async function withApp(kind,run){const root=resolve(`components/widgetjs/${kind}-source`),vite=await createServer({configFile:resolve('components/widgetjs/calendar-source/vite.config.mjs'),root,server:{middlewareMode:true,hmr:false,ws:false,watch:null},appType:'custom',logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]}});try{const{default:App}=await vite.ssrLoadModule('/App.vue');const model=kind==='pomodoro'?await vite.ssrLoadModule('/pomodoro-model.js'):null;await run(App,model);}finally{await vite.close();}}
async function render(App,props){let state;const Subject={...App,setup(props,context){state=App.setup(props,context);return state;}};await renderToString(createSSRApp(Subject,props));return state;}
const dialog=()=>({open:false,showModal(){this.open=true;},close(){this.open=false;}});
test('待办和番茄删除确认明确共享库会影响全部连接实例',async()=>{
  for(const kind of['todo','pomodoro'])await withApp(kind,async(App,model)=>{const initialSnapshot=kind==='todo'?{version:1,revision:0,sampledAt:0,todos:[],reminders:[]}:model.emptySnapshot(),props={host:{preview:true},initialSnapshot,...(kind==='todo'?{initialSettings:{title:'待办',background:'#000',foreground:'#fff',divider:'#aaa',radius:22}}:{})};const html=await renderToString(createSSRApp(App,props));assert.ok(!html.includes('此操作删除本实例中的事项'));assert.match(html,/共享库[^<]*连接实例/);});
});
test('待办切源钩子阻止未确定操作与设置草稿窗口',async()=>withApp('todo',async(App)=>{
  const host={preview:false,request(){throw Error('不应写入');}},initialSnapshot={version:1,revision:0,sampledAt:0,todos:[],reminders:[]};const state=await render(App,{host,initialSnapshot,initialSettings:{title:'待办',background:'#000',foreground:'#fff',divider:'#aaa',radius:22}});assert.equal(typeof host.beforeLibrarySwitch,'function');state.uncertain.value=true;await assert.rejects(host.beforeLibrarySwitch(),/未确定|重试/);state.uncertain.value=false;state.themeDialog.value=dialog();state.themeDialog.value.open=true;await assert.rejects(host.beforeLibrarySwitch(),/窗口|设置/);state.store.dispose();
}));
test('大号日历的待办编辑器将切源保护转接到同一宿主',async()=>withApp('calendar-large',async(App)=>{
  const host={preview:true},initialSnapshot={version:1,revision:0,sampledAt:0,todos:[],reminders:[]};const state=await render(App,{host,initialSnapshot,initialSettings:{background:'#000',foreground:'#fff',radius:22,firstDayOfWeek:1}});assert.equal(typeof host.beforeLibrarySwitch,'function');assert.equal(state.editorHost.beforeLibrarySwitch,host.beforeLibrarySwitch);state.editorHost.beforeLibrarySwitch=undefined;assert.equal(host.beforeLibrarySwitch,undefined);
}));
test('番茄配置未保存时不能切源，休息暂停仍显示继续休息动作',async()=>withApp('pomodoro',async(App,model)=>{
  const sceneId=id,initialSnapshot={...model.emptySnapshot(),initialized:true,scenes:[{id:sceneId,name:'阅读',icon:'📖',sortOrder:0}],currentSceneId:sceneId,phase:{status:'rest-pause',cycleId:id,sceneId:id,startedAt:1000,phaseStartedAt:null,focusMillis:70000,restMillis:10000},remainingSeconds:50};const host={preview:false,request(){throw Error('不应写入');}};
  const state=await render(App,{host,initialSnapshot,initialPage:'pomodoro-settings'});assert.equal(typeof host.beforeLibrarySwitch,'function');state.configDraft.value.pomoTime=35;await assert.rejects(host.beforeLibrarySwitch(),/配置|设置|保存/);state.store.dispose();const html=await renderToString(createSSRApp(App,{host:{preview:true},initialSnapshot}));assert.match(html,/继续休息/);
}));

test('待办旧草稿冲突后不借自动重读的新版本覆盖他人，重新打开后才可保存',async()=>withApp('todo',async App=>{
  const initialSnapshot={version:1,revision:1,sampledAt:0,todos:[{id,title:'原事项',isReminderOn:false,recurrence:'',importance:'normal',order:0,createdDateTime:'2026-11-15T00:00:00.000Z',lastModifiedDateTime:'2026-11-15T00:00:00.000Z'}],reminders:[]};
  let current=structuredClone(initialSnapshot);const attempts=[];const host={preview:false,async request(method,args){if(method==='todo.get')return structuredClone(current);assert.equal(method,'todo.change');attempts.push(args.expectedRevision);if(args.expectedRevision!==current.revision)throw Error('REVISION_CONFLICT: 其他实例已修改');current={...current,revision:current.revision+1,todos:[args.todo]};return structuredClone(current);}};
  const state=await render(App,{host,initialSnapshot,initialSettings:{title:'待办',background:'#000',foreground:'#fff',divider:'#aaa',radius:22}});state.editor.value=dialog();state.confirm.value=dialog();state.themeDialog.value=dialog();
  state.openEdit(id);state.draft.value.title='我的旧草稿';current={...current,revision:2,todos:[{...current.todos[0],title:'他人新修改'}]};
  await state.saveTodo();await state.saveTodo();assert.deepEqual(attempts,[1,1]);assert.equal(current.todos[0].title,'他人新修改');assert.equal(state.draft.value.title,'我的旧草稿');assert.equal(state.editor.value.open,true);
  state.editor.value.close();state.openEdit(id);state.draft.value.title='审阅后修改';await state.saveTodo();assert.equal(current.todos[0].title,'审阅后修改');assert.equal(state.editor.value.open,false);state.store.dispose();
}));

test('番茄钟场景与配置草稿固定打开时版本，冲突重读不能自动放行旧草稿',async()=>withApp('pomodoro',async(App,model)=>{
  const initialSnapshot={...model.emptySnapshot(),initialized:true,revision:1,scenes:[{id,name:'原场景',icon:'📖',sortOrder:0}],currentSceneId:id};let current=structuredClone(initialSnapshot);const attempts=[];
  const host={preview:false,async request(method,args){if(method==='pomodoro.get')return structuredClone(current);assert.equal(method,'pomodoro.change');attempts.push([args.operation,args.expectedRevision]);if(args.expectedRevision!==current.revision)throw Error('REVISION_CONFLICT: 其他实例已修改');current={...current,revision:current.revision+1,...(args.operation==='configure'?{config:args.config}:{scenes:[{...current.scenes[0],...args.scene}]})};return structuredClone(current);}};
  const state=await render(App,{host,initialSnapshot,initialPage:'pomodoro-scenes'});state.editor.value=dialog();
  state.openScene(state.snapshot.value.scenes[0]);state.draft.value.name='旧场景草稿';current={...current,revision:2,scenes:[{...current.scenes[0],name:'他人新场景'}]};await state.saveScene();await state.saveScene();assert.deepEqual(attempts,[['scene.save',1],['scene.save',1]]);assert.equal(current.scenes[0].name,'他人新场景');assert.equal(state.editor.value.open,true);
  state.editor.value.close();state.openScene(state.snapshot.value.scenes[0]);state.draft.value.name='审阅后场景';await state.saveScene();assert.equal(current.scenes[0].name,'审阅后场景');
  state.navigate('pomodoro-settings');const revision=current.revision;state.configDraft.value.pomoTime=35;current={...current,revision:revision+1,config:{...current.config,pomoTime:40}};await state.saveConfig();await state.saveConfig();assert.deepEqual(attempts.slice(-2),[['configure',revision],['configure',revision]]);assert.equal(current.config.pomoTime,40);
  state.navigate('clock');state.navigate('pomodoro-settings');state.configDraft.value.pomoTime=45;await state.saveConfig();assert.equal(current.config.pomoTime,45);state.configDraft.value.pomoTime=46;await state.saveConfig();assert.equal(current.config.pomoTime,46);state.store.dispose();
}));

test('待办删除确认固定打开版本，冲突后须重新打开确认',async()=>withApp('todo',async App=>{
  const initialSnapshot={version:1,revision:1,sampledAt:0,todos:[{id,title:'原事项',isReminderOn:false,recurrence:'',importance:'normal',order:0,createdDateTime:'2026-11-15T00:00:00.000Z',lastModifiedDateTime:'2026-11-15T00:00:00.000Z'}],reminders:[]};let current=structuredClone(initialSnapshot);const attempts=[];
  const host={preview:false,async request(method,args){if(method==='todo.get')return structuredClone(current);attempts.push(args.expectedRevision);if(args.expectedRevision!==current.revision)throw Error('REVISION_CONFLICT: 他人修改');current={...current,revision:current.revision+1,todos:[]};return structuredClone(current);}};
  const state=await render(App,{host,initialSnapshot,initialSettings:{title:'待办',background:'#000',foreground:'#fff',divider:'#aaa',radius:22}});state.confirm.value=dialog();state.askDelete(id);current={...current,revision:2,todos:[{...current.todos[0],title:'他人修改后'}]};await state.remove();await state.remove();assert.deepEqual(attempts,[1,1]);assert.equal(current.todos.length,1);state.confirm.value.close();state.askDelete(id);await state.remove();assert.equal(current.todos.length,0);state.store.dispose();
}));

test('番茄钟场景和历史删除都固定确认时版本，不借冲突重读删除他人内容',async()=>withApp('pomodoro',async(App,model)=>{
  for(const type of ['scene','history']){const historyId='1f282c1b-9bc8-461a-87cc-98de36e334a7',initialSnapshot={...model.emptySnapshot(),initialized:true,revision:1,scenes:[{id,name:'原场景',icon:'📖',sortOrder:0}],currentSceneId:id,history:[{id:historyId,sceneId:id,startTime:1000,finishTime:61000,duration:60}]};let current=structuredClone(initialSnapshot);const attempts=[];
    const host={preview:false,async request(method,args){if(method==='pomodoro.get')return structuredClone(current);attempts.push(args.expectedRevision);if(args.expectedRevision!==current.revision)throw Error('REVISION_CONFLICT: 他人修改');current={...current,revision:current.revision+1,history:[],...(type==='scene'?{scenes:[],currentSceneId:null}:{})};return structuredClone(current);}};
    const state=await render(App,{host,initialSnapshot,initialPage:'pomodoro-scenes'});state.confirmation.value=dialog();const target=type==='scene'?id:historyId;state.askDelete(type,target);current={...current,revision:2,scenes:[{...current.scenes[0],name:'他人新场景'}]};await state.remove();await state.remove();assert.deepEqual(attempts,[1,1],type);assert.equal(current.history.length,1,type);state.confirmation.value.close();state.askDelete(type,target);await state.remove();assert.equal(current.history.length,0);state.store.dispose();
  }
}));

test('番茄配置原UUID重试成功后显示实际最新配置并推进基线，同页可继续保存',async()=>withApp('pomodoro',async(App,model)=>{
  const initialSnapshot={...model.emptySnapshot(),initialized:true,revision:1};let current=structuredClone(initialSnapshot),first;const calls=[];
  const host={preview:false,async request(method,args){assert.equal(method,'pomodoro.change');calls.push(structuredClone(args));if(!first){first=structuredClone(args);current={...current,revision:2,config:args.config};throw Error('请求超时');}if(args.requestId===first.requestId)return structuredClone(current);if(args.expectedRevision!==current.revision)throw Error('REVISION_CONFLICT: 配置已更新');current={...current,revision:current.revision+1,config:args.config};return structuredClone(current);}};
  const state=await render(App,{host,initialSnapshot,initialPage:'pomodoro-settings'});state.configDraft.value.pomoTime=35;await state.saveConfig();assert.equal(state.uncertain.value,true);current={...current,revision:3,config:{...current.config,pomoTime:40}};await state.retry();assert.deepEqual(calls[0],calls[1]);assert.equal(state.configDraft.value.pomoTime,40);state.configDraft.value.pomoTime=45;await state.saveConfig();assert.equal(calls.at(-1).expectedRevision,3);assert.equal(current.config.pomoTime,45);state.store.dispose();
}));

test('番茄配置保存期间离开再返回，成功回执同步实际草稿，不把旧配置配新版本',async()=>withApp('pomodoro',async(App,model)=>{
  const initialSnapshot={...model.emptySnapshot(),initialized:true,revision:1};let current=structuredClone(initialSnapshot),finish,start;const started=new Promise(resolve=>{start=resolve;}),calls=[];
  const host={preview:false,async request(method,args){assert.equal(method,'pomodoro.change');calls.push(structuredClone(args));if(calls.length===1){start();await new Promise(resolve=>{finish=resolve;});}current={...current,revision:current.revision+1,config:args.config};return structuredClone(current);}};
  const state=await render(App,{host,initialSnapshot,initialPage:'pomodoro-settings'});state.configDraft.value.pomoTime=35;const saving=state.saveConfig();await started;state.navigate('pomodoro-scenes');state.navigate('pomodoro-settings');assert.equal(state.configDraft.value.pomoTime,30);finish();await saving;assert.equal(state.configDraft.value.pomoTime,35);await state.saveConfig();assert.equal(calls.at(-1).expectedRevision,2);assert.equal(current.config.pomoTime,35);state.store.dispose();
}));

test('番茄专用窗许可冲突保留开关草稿和原版本，主动载入后才使用新基线',async()=>withApp('pomodoro',async(App,model)=>{
  const initialWindow={revision:1,config:{enabled:true},surface:null,nativeCleanupPending:false,nativeError:null};let actual={...initialWindow,revision:2},attempts=[];
  const host={preview:false,async request(method,args){if(method==='pomodoro.window.status')return structuredClone(actual);assert.equal(method,'pomodoro.window.configure');attempts.push(args.expectedRevision);if(args.expectedRevision!==actual.revision)throw Error('REVISION_CONFLICT: 其他窗口已修改');actual={...actual,revision:actual.revision+1,config:args.config};return structuredClone(actual);}};
  const state=await render(App,{host,initialSnapshot:model.emptySnapshot(),initialWindow,initialPage:'pomodoro-settings'});state.windowDraft.value=false;
  await state.saveWindow();await state.saveWindow();assert.deepEqual(attempts,[1,1]);assert.equal(state.windowDraft.value,false);assert.equal(actual.config.enabled,true);
  await state.reloadWindowDraft();assert.equal(state.windowDraft.value,true);state.windowDraft.value=false;await state.saveWindow();assert.deepEqual(attempts,[1,1,2]);assert.equal(actual.config.enabled,false);state.store.dispose();state.windowControl.dispose();
}));

test('共享计时损坏时停用专用窗只写私有开关，不读写错误的业务源',async()=>withApp('pomodoro',async(App,model)=>{const calls=[],initialWindow={revision:4,config:{enabled:true},surface:null,nativeCleanupPending:false,nativeError:null};const host={preview:false,async request(method,args){calls.push([method,args]);assert.equal(method,'pomodoro.window.configure');return{...initialWindow,revision:5,config:args.config};}};const state=await render(App,{host,initialSnapshot:model.emptySnapshot(),initialWindow,initialError:'DATA_UNAVAILABLE: 共享计时损坏'});await state.stopUnavailableWindow();assert.equal(calls.length,1);assert.equal(calls[0][1].expectedRevision,4);assert.deepEqual(calls[0][1].config,{enabled:false});assert.equal(state.windowState.value.config.enabled,false);state.store.dispose();state.windowControl.dispose();}));
