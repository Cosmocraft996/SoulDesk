import test from'node:test';import assert from'node:assert/strict';
import{createAssistantClient,defaultAssistantConfig}from'./assistant-client.js';
const copy=v=>JSON.parse(JSON.stringify(v));
function fixture(){
  const value={revision:0,config:defaultAssistantConfig(),surface:null},calls=[];
  let fail,hold;
  const host={request:async(method,args)=>{
    calls.push([method,copy(args??null)]);
    if(hold&&method==='assistant.status')return hold;
    if(fail)throw fail;
    if(method==='assistant.configure'){
      if(args.expectedRevision!==value.revision)throw Error('REVISION_CONFLICT: 已有新版本');
      value.revision++;value.config=copy(args.config);
    }else if(method==='assistant.overlay'){
      value.surface={presentation:'test-presentation',serial:(value.surface?.serial??0)+1,phase:'ready',nativeSettled:false,overlays:args.visible?[args.id]:[]};
    }
    return copy(value);
  }};
  return{host,calls,value,fail:e=>fail=e,hold:p=>hold=p};
}
test('网页助手默认后台/置顶关闭，预览不请求网页宿主或自动打开',async()=>{
  let calls=0;const client=createAssistantClient({preview:true,request:()=>{calls++;}});
  await client.load();await client.action('open');await client.save(defaultAssistantConfig());
  assert.equal(calls,0);assert.equal(client.state.config.backgroundEnabled,false);assert.equal(client.state.config.alwaysTop,false);
  assert.equal(client.state.surface,null);client.dispose();
});
test('轮询不自动打开、保存或调用任意浏览器方法',async()=>{
  const f=fixture(),c=createAssistantClient(f.host);await c.load();await c.refresh();
  assert.deepEqual(f.calls.map(v=>v[0]),['assistant.status','assistant.status']);
  await assert.rejects(()=>c.action('eval'),/CAPABILITY_DENIED/);await assert.rejects(()=>c.overlay('settings',true),/CAPABILITY_DENIED/);
  assert.equal(f.calls.length,2);c.dispose();
});
test('保存失败保留草稿和成功配置，连续成功保存使用新版本',async()=>{
  const f=fixture(),c=createAssistantClient(f.host);await c.load();const draft={...defaultAssistantConfig(),pageCount:2};
  f.fail(Error('STORAGE_ERROR: 保存失败'));assert.equal(await c.save(draft),false);assert.equal(c.state.config.pageCount,1);assert.equal(draft.pageCount,2);
  f.fail(undefined);assert.equal(await c.save(draft),true);assert.equal(await c.quick({alwaysTop:true}),true);
  assert.deepEqual(f.calls.filter(v=>v[0]==='assistant.configure').map(v=>v[1].expectedRevision),[0,0,1]);assert.equal(c.state.revision,2);c.dispose();
});
test('保存超时只重试原UUID，禁止把新草稿变成另一次请求',async()=>{
  const f=fixture(),c=createAssistantClient(f.host);await c.load();const draft={...defaultAssistantConfig(),pageCount:4};
  f.fail(Error('HOST_TIMEOUT: 保存结果未确定'));await c.save(draft);assert.equal(c.state.uncertain,true);
  await c.save({...draft,pageCount:2});assert.equal(f.calls.filter(v=>v[0]==='assistant.configure').length,1);
  f.fail(undefined);assert.equal(await c.retrySave(),true);const saved=f.calls.filter(v=>v[0]==='assistant.configure');assert.deepEqual(saved[0][1],saved[1][1]);assert.equal(c.state.uncertain,false);c.dispose();
});
test('远端新配置不能成为旧设置草稿的保存基线',async()=>{
  const f=fixture(),c=createAssistantClient(f.host);await c.load();const baseline=c.state.revision,draft={...c.state.config,widthRatio:0.4};
  f.value.revision=3;f.value.config.pageCount=4;await c.refresh();
  assert.equal(await c.save(draft,baseline),false);assert.equal(f.calls.at(-1)[1].expectedRevision,0);assert.equal(c.state.config.pageCount,4);assert.equal(draft.widthRatio,0.4);c.dispose();
});
test('未知保存实际提交后被其他窗口更新，原UUID回读不把旧草稿冒称当前已保存',async()=>{
  let calls=0;const current={revision:0,config:defaultAssistantConfig(),surface:null};
  const host={request:async method=>{if(method==='assistant.configure'){calls++;if(calls===1){current.revision=1;current.config.pageCount=2;throw Error('HOST_TIMEOUT: 回包未确定');}return{...copy(current),replayed:true};}return copy(current);}};
  const c=createAssistantClient(host);await c.load();const draft={...defaultAssistantConfig(),pageCount:2};await c.save(draft,0);
  current.revision=2;current.config.widthRatio=0.5;
  assert.equal(await c.retrySave(),false);assert.equal(c.state.uncertain,false);assert.equal(c.state.config.widthRatio,0.5);assert.equal(draft.widthRatio,0.3);assert.match(c.state.error,/随后有新改动/);c.dispose();
});
test('开始保存前发出的迟到状态不能清空新配置或重新打开旧显示',async()=>{
  const f=fixture(),c=createAssistantClient(f.host);await c.load();let resolve;
  f.hold(new Promise(r=>resolve=r));const old=c.refresh();
  assert.equal(await c.quick({pageCount:2}),true);resolve({revision:0,config:defaultAssistantConfig(),surface:{presentation:'old',serial:1}});await old;
  assert.equal(c.state.config.pageCount,2);assert.equal(c.state.surface,null);c.dispose();
});
test('原生网页确认隐藏后才允许展开覆盖层，设置关闭也等待确认',async()=>{
  const f=fixture();let waits=0;const c=createAssistantClient(f.host,{shell:true,sleep:async()=>{waits++;f.value.surface.nativeSettled=true;}});await c.load();
  assert.equal(await c.overlay('settings',true),true);assert.equal(waits,1);assert.equal(c.state.surface.nativeSettled,true);
  assert.equal(await c.overlay('settings',false),true);assert.equal(waits,2);assert.deepEqual(c.state.surface.overlays,[]);c.dispose();
});
test('覆盖层未确认不报告展开成功，释放后的迟到结果不再通知界面',async()=>{
  const f=fixture();let updates=0;const c=createAssistantClient(f.host,{shell:true,sleep:async()=>{},changed:()=>updates++});await c.load();
  assert.equal(await c.overlay('select0',true),false);assert.match(c.state.error,/HOST_TIMEOUT/);
  const n=updates;c.dispose();await c.refresh();assert.equal(updates,n);
});
test('旧本机回包不覆盖同版本更新的展示状态，损坏状态保留画面',async()=>{
  const f=fixture(),c=createAssistantClient(f.host);f.value.surface={presentation:'new',serial:9,phase:'ready'};await c.load();
  f.value.surface={presentation:'old',serial:3,phase:'opening'};await c.refresh();assert.equal(c.state.surface.presentation,'new');
  f.value.config.platforms=['invalid'];await c.refresh();assert.equal(c.state.surface.presentation,'new');assert.match(c.state.pollError,/DATA_UNAVAILABLE/);c.dispose();
});
