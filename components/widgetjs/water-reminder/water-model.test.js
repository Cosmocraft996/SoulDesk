import test from 'node:test';
import assert from 'node:assert/strict';
import { createWater,waveProgress } from './water-model.js';
const snapshot=(count=0,day='2026-11-15')=>({sampledAt:1000,day,count,targetCup:8,enableReminder:true,interval:30,lastReminderAt:1000,nextReminderAt:1801000,delivery:'',lastError:null});
test('杯数更新失败后保留原视图，重试相同请求标识，不重复喝一杯',async()=>{
  const calls=[];let fail=true;
  const model=createWater(async(method,args)=>{calls.push([method,args]);if(method==='water.get')return snapshot();if(fail){fail=false;throw new Error('响应丢失');}return snapshot(1);},()=>{},()=> 'fixed-request');
  assert.equal(await model.refresh(),true);assert.equal(await model.change('add'),false);assert.equal(model.state.snapshot.count,0);assert.match(model.state.error,/响应丢失/);
  assert.equal(await model.change('reset'),false);assert.equal(calls.length,2);assert.equal(await model.retry(),true);assert.deepEqual(calls[1],calls[2]);assert.deepEqual(calls[2],['water.change',{requestId:'fixed-request',operation:'add'}]);assert.equal(model.state.snapshot.count,1);assert.equal(model.state.pending,null);assert.equal(model.state.error,null);
});
test('读取与修改串行，离开视图忽略迟到结果',async()=>{
  let finish,calls=0;const model=createWater(()=>{calls++;return new Promise(resolve=>finish=resolve);});
  const first=model.refresh();assert.equal(await model.refresh(),false);assert.equal(await model.change('add'),false);assert.equal(calls,1);finish(snapshot(3));await first;assert.equal(model.state.snapshot.count,3);
  const late=model.refresh();model.dispose();finish(snapshot(8));await late;assert.equal(model.state.snapshot.count,3);
});
test('按宿主返回日期跨日，异常杯数不冒充零和成功',async()=>{
  let result=snapshot(5);const model=createWater(async()=>result);await model.refresh();result=snapshot(0,'2026-11-16');await model.refresh();assert.equal(model.state.snapshot.day,'2026-11-16');assert.equal(model.state.snapshot.count,0);
  result={...snapshot(),count:-1};assert.equal(await model.refresh(),false);assert.equal(model.state.snapshot.day,'2026-11-16');assert.match(model.state.error,/数据/);assert.equal(await model.change('invalid'),false);
  assert.equal(waveProgress(0,8),0);assert.equal(waveProgress(4,8),50);assert.equal(waveProgress(9,8),100);assert.equal(waveProgress(null,8),null);assert.equal(waveProgress(1,0),null);
});
