import test from 'node:test';
import assert from 'node:assert/strict';
import { energy } from './energy-model.js';
const snapshot=(cpu,memory,at=10000)=>({sampledAt:at,cpu:{percent:cpu,error:cpu===null?'CPU正在采样':null},memory:{percent:memory,error:memory===null?'内存不可用':null},gpu:{percent:null,error:'GPU不参与分级'}});
test('能耗标签保留原80/20权重、五档边界和36像素步进',()=>{
  const first=energy(snapshot(0,0),10000);
  assert.equal(first.level,1);assert.equal(first.score,0);assert.equal(first.color,'#1b814a');assert.equal(first.top,0);
  assert.equal(energy(snapshot(25,0),10000).level,1);
  assert.equal(energy(snapshot(25.01,0),10000).level,2);
  assert.equal(energy(snapshot(0,100),10000).level,1);
  const full=energy(snapshot(100,100),10000);assert.equal(full.level,5);assert.equal(full.score,100);assert.equal(full.top,144);assert.equal(full.color,'#e30f1b');
  assert.equal(energy(snapshot(89,82),10000).level,5);
});
test('预热、缺值、过期和未来读数均不冒充绿色一级；单项有效值仍可辨认',()=>{
  const missing=energy(snapshot(null,67),10000);assert.equal(missing.level,null);assert.equal(missing.cpuText,'—');assert.equal(missing.memoryText,'67%');assert.match(missing.status,/采样/);
  assert.equal(energy(null,10000).level,null);
  assert.equal(energy(snapshot(0,0),22000).level,null);
  assert.equal(energy(snapshot(0,0),0).level,null);
  assert.equal(energy(snapshot(101,0),10000).level,null);
});
