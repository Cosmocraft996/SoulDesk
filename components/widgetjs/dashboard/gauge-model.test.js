import test from 'node:test';
import assert from 'node:assert/strict';
import { gauge, createMetrics } from './gauge-model.js';
const snapshot=(at=10000)=>({sampledAt:at,cpu:{percent:50,error:null},memory:{percent:0,error:null},gpu:{percent:null,error:'驱动未提供计数器'}});
test('仪表盘只显示对应指标的有效占用率，零值有效，预热与旧数据不冒充零',()=>{
  assert.deepEqual(gauge(snapshot(),'cpu',11000),{label:'CPU 50%',angle:-40,status:'',valid:true});
  assert.equal(gauge(snapshot(),'memory',11000).label,'Mem 0%');
  assert.equal(gauge(snapshot(),'gpu',11000).label,'GPU —');
  assert.match(gauge(snapshot(),'gpu',11000).status,/驱动/);
  assert.match(gauge(snapshot(),'cpu',22000).status,/过期/);
  assert.equal(gauge(snapshot(),'cpu',0).valid,false);
  assert.equal(gauge(null,'cpu',11000).valid,false);
});
test('监控轮询拒绝倒退与畸形读数，保留上次有效快照且卸载后不再更新',async()=>{
  let answer=snapshot(),pending;
  const model=createMetrics(()=>new Promise(resolve=>{pending=resolve;}),()=>{});
  let request=model.refresh();pending(answer);await request;
  assert.deepEqual(model.state.snapshot,answer);
  request=model.refresh();pending(snapshot(9000));await request;
  assert.equal(model.state.snapshot.sampledAt,10000);assert.match(model.state.error,/倒退/);
  request=model.refresh();pending({...snapshot(12000),cpu:{percent:101,error:null}});await request;
  assert.equal(model.state.snapshot.sampledAt,10000);assert.match(model.state.error,/无效/);
  request=model.refresh();model.dispose();pending(snapshot(13000));await request;
  assert.equal(model.state.snapshot.sampledAt,10000);
});
