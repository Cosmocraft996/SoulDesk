import test from'node:test';import assert from'node:assert/strict';import{deadline,defaults,initialSettings,loadSettings,localDay,settingsFromDraft,validateSettings}from'./deadline-model.js';
const at=text=>Date.parse(text),base={...defaults,startTime:'2026-11-14T00:00:00.000Z',endTime:'2026-11-16T00:00:00.000Z'};
test('期限每分钟按原整数分钟口径推进，原SVG位移在开始/中点/结束后正确',()=>{
  assert.deepEqual(deadline(base,at('2026-11-13T23:59:00Z')),{remainingDays:2,percent:0,fillX:-100,deathX:110});
  assert.deepEqual(deadline(base,at('2026-11-15T00:00:00Z')),{remainingDays:1,percent:50,fillX:-50,deathX:315});
  const next=deadline(base,at('2026-11-15T00:30:00Z'));assert.equal(next.deathX,319);assert.ok(next.fillX>-50);
  assert.deepEqual(deadline(base,at('2026-11-17T00:00:00Z')),{remainingDays:-1,percent:100,fillX:0,deathX:520});
});
test('日期不是有效ISO、相同/倒序、短于一分钟及非法主题不冒充进度',()=>{
  for(const startTime of ['x','2026-02-30T00:00:00.000Z','2026-11-14'])assert.throws(()=>validateSettings({...base,startTime}),/日期/);
  for(const endTime of [base.startTime,'2026-11-13T00:00:00.000Z','2026-11-14T00:00:59.999Z'])assert.throws(()=>validateSettings({...base,endTime}),/至少一分钟/);
  assert.throws(()=>validateSettings({...base,radius:51}),/圆角/);assert.throws(()=>validateSettings({...base,primaryColor:'url(https://example.test)'}),/颜色/);
  assert.throws(()=>deadline(base,NaN),/当前时间/);
});
test('首次默认采用本地昨天至明天，并且仅正式实例保存一次；预览不写入',async()=>{
  let stored={...defaults},writes=0;const host={preview:false,async request(method,args){if(method==='settings.get')return structuredClone(stored);if(method==='settings.update'){writes++;stored=structuredClone(args);return stored;}throw Error(method);}};
  const now=at('2026-11-15T04:37:12.500Z'),first=await loadSettings(host,now),later=await loadSettings(host,now+86400000);assert.equal(writes,1);assert.deepEqual(first,later);
  const yesterday=new Date(now);yesterday.setDate(yesterday.getDate()-1);assert.equal(first.startTime,yesterday.toISOString());
  host.preview=true;stored={...defaults};await loadSettings(host,now);assert.equal(writes,1);assert.equal(stored.startTime,'');
  host.preview=false;host.request=async method=>{if(method==='settings.get')return{...defaults};throw Error('磁盘只读');};await assert.rejects(loadSettings(host,now),/磁盘只读/);
  assert.throws(()=>initialSettings({...defaults,startTime:base.startTime},now),/日期/);
});
test('只改主题不截断已有时分；本地日期输入跨年正确且拒绝不存在日期',()=>{
  const value={...base,startTime:'2026-11-14T04:37:12.500Z',endTime:'2026-11-16T04:37:12.500Z'};
  const draft={title:'交付',startDate:localDay(value.startTime),endDate:localDay(value.endTime),primaryColor:'#BE002A',background:'rgba(0,0,0,0.5)',radius:10};
  const next=settingsFromDraft(value,draft);assert.equal(next.startTime,value.startTime);assert.equal(next.endTime,value.endTime);
  const moved=settingsFromDraft(value,{...draft,startDate:'2026-12-31',endDate:'2027-01-02'});assert.equal(localDay(moved.startTime),'2026-12-31');assert.equal(localDay(moved.endTime),'2027-01-02');assert.equal(new Date(moved.startTime).getHours(),0);
  assert.throws(()=>settingsFromDraft(value,{...draft,endDate:'2026-02-30'}),/有效日期/);
});
