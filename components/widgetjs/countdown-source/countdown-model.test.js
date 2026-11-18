import test from'node:test';import assert from'node:assert/strict';import{existsSync}from'node:fs';
const modelPath=new URL('./countdown-model.js',import.meta.url);
async function model(){assert.ok(existsSync(modelPath),'倒计时本地模型尚未实现');return await import(modelPath);}
const date=(y,m,d,h=0)=>new Date(y,m-1,d,h),event=(extra={})=>({id:'a',name:'目标',note:'备注',sourceDateTime:date(2026,11,16).toISOString(),dateTime:date(2026,11,16).toISOString(),dateType:0,recurrence:'',sortOrder:0,createdAt:1,updatedAt:1,...extra});
test('日数沿源向上取整，列表当天为零、独立卡片过去当天保留原计数口径',async()=>{
  const m=await model();assert.equal(m.viewEvent(event(),date(2026,11,15,12).getTime()).days,1);assert.equal(m.viewEvent(event(),date(2026,11,16,12).getTime()).days,0);assert.equal(m.viewEvent(event(),date(2026,11,17,12).getTime()).days,-1);
  const v=m.viewEvent(event(),date(2026,11,15,12).getTime());assert.equal(v.dateText,'2026年11月16日');assert.equal(v.cardDateText,'2026/11/16');assert.equal(v.today,false);assert.equal(v.past,false);
});
test('公历重复调用固定原RRule算法，月末跳过不存在日且不跳过今天',async()=>{
  const m=await model();const value=event({sourceDateTime:date(2026,1,31).toISOString(),dateTime:date(2026,1,31).toISOString(),recurrence:'RRULE:FREQ=MONTHLY'});
  const result=m.viewEvent(value,date(2026,2,15,12).getTime());assert.equal(result.cardDateText,'2026/03/31');assert.equal(result.days,44);
  const daily=m.viewEvent(event({sourceDateTime:date(2026,11,1).toISOString(),recurrence:'RRULE:FREQ=DAILY'}),date(2026,11,16,12).getTime());assert.equal(daily.today,true);assert.equal(daily.cardDateText,'2026/11/16');assert.equal(daily.days,0);
  const first=m.viewEvent(event({sourceDateTime:date(2026,1,1).toISOString(),recurrence:'RRULE:FREQ=MONTHLY'}),date(2026,2,15,12).getTime());assert.equal(first.cardDateText,'2026/03/01','每月1日不能受UTC上一日31号影响而跳过3月');
});
test('农历日期沿原模块转换，目标公历与原农历年日期同时保持',async()=>{
  const m=await model(),draft={name:'春节',note:'团圆',dateType:1,year:2027,month:1,dayOfMonth:1,recurrence:''};
  const created=m.eventFromDraft(draft,null,1,()=> 'spring');
  const result=m.viewEvent(created,date(2027,2,5,12).getTime());
  assert.equal(result.cardDateText,'二〇二七年正月初一');
  assert.equal(result.dateTime,date(2027,2,6).toISOString());
  assert.equal(result.days,1);
  assert.throws(()=>m.eventFromDraft({...draft,year:2026,month:-2},null,1,()=> 'x'),/农历/);
});
test('名称15字、日期/重复/唯一性/排序及存储字节限制失败不改原值',async()=>{
  const m=await model();assert.throws(()=>m.validateDocument({version:1,events:[event(),event()],selectedId:'a',sort:'asc'}),/重复/);assert.throws(()=>m.validateDocument({version:1,events:[event({name:'a'.repeat(16)})],selectedId:'a',sort:'asc'}),/15/);
  for(const recurrence of['bad','RRULE:FREQ=SECONDLY','RRULE:FREQ=DAILY;INTERVAL=0'])assert.throws(()=>m.validateDocument({version:1,events:[event({recurrence})],selectedId:'a',sort:'asc'}),/重复/);
  assert.throws(()=>m.eventFromDraft({name:' ',note:'',dateType:0,date:'2026-11-16',recurrence:''},null,1,()=> 'x'),/名称/);assert.throws(()=>m.eventFromDraft({name:'x',note:'',dateType:0,date:'2026-02-30',recurrence:''},null,1,()=> 'x'),/日期/);
  assert.throws(()=>m.validateDocument({version:1,events:[event({note:'a'.repeat(5000)})],selectedId:'a',sort:'asc'},100),/备注|上限/);
});
test('新年默认只在空实例首次生成；清空后重开保持空列表，不复活默认',async()=>{
  const m=await model();const a=m.restoreDocument({},date(2026,11,15).getTime(),()=> 'newyear');assert.equal(a.events[0].name,'新年');assert.equal(m.viewEvent(a.events[0],date(2026,11,15).getTime()).cardDateText,'2027/01/01');assert.equal(a.selectedId,'newyear');
  assert.equal(m.restoreDocument({version:1,events:[],selectedId:'',sort:'asc'}).events.length,0);assert.throws(()=>m.restoreDocument({version:9}),/存储/);
});
test('串行保存成功才应用；写入失败保留原值，后续操作不被失败链锁死',async()=>{
  const m=await model(),first={version:1,events:[event()],selectedId:'a',sort:'asc'},written=[];let release,fail=true;
  const store=m.createStore(first,async next=>{written.push(structuredClone(next));if(fail){await new Promise(resolve=>{release=resolve;});throw Error('磁盘只读');}});
  const failed=store.update(doc=>{doc.events[0].name='未保存';});await new Promise(resolve=>setTimeout(resolve,0));assert.equal(store.get().events[0].name,'目标');release();await assert.rejects(failed,/磁盘只读/);assert.equal(store.get().events[0].name,'目标');fail=false;await store.update(doc=>{doc.events[0].name='已保存';});assert.equal(store.get().events[0].name,'已保存');assert.equal(written.length,2);
});
test('选择、删除、拖动顺序与日期升降序保持，预览不写入且关闭后拒绝新操作',async()=>{
  const m=await model();const doc={version:1,events:[event(),event({id:'b',name:'更早',sourceDateTime:date(2026,11,14).toISOString(),dateTime:date(2026,11,14).toISOString()})],selectedId:'a',sort:'asc'};
  assert.deepEqual(m.ordered(doc,date(2026,11,15).getTime()).map(x=>x.id),['b','a']);assert.deepEqual(m.ordered({...doc,sort:'desc'},date(2026,11,15).getTime()).map(x=>x.id),['a','b']);assert.deepEqual(m.reorder(doc,'a','b').events.map(x=>[x.id,x.sortOrder]),[['a',0],['b',1]]);
  const store=m.createStore(doc,async()=>{throw Error('不应写入');},{preview:true});await assert.rejects(store.update(()=>{}),/预览/);assert.deepEqual(store.get(),doc);store.dispose();await assert.rejects(store.update(()=>{}),/关闭/);
});

test('切换共享库前可等待全部已提交保存，不能提前重载丢失写入',async()=>{const m=await model();let finish,start,flushed=false;const started=new Promise(resolve=>{start=resolve;});const initial={version:1,events:[],selectedId:'',sort:'asc'};const store=m.createStore(initial,()=>{start();return new Promise(resolve=>{finish=resolve;});});const updating=store.update(()=>{});await started;const waiting=store.flush().then(()=>{flushed=true;});await Promise.resolve();assert.equal(flushed,false);finish();await updating;await waiting;assert.equal(flushed,true);store.dispose();});

test('上次保存失败时flush继续拒绝共享库切换，不能吞掉原错误',async()=>{const m=await model();const initial={version:1,events:[],selectedId:'',sort:'asc'};const store=m.createStore(initial,async()=>{throw Error('磁盘只读');});await assert.rejects(store.update(()=>{}),/磁盘只读/);await assert.rejects(store.flush(),/磁盘只读/);assert.deepEqual(store.get(),initial);store.dispose();});
