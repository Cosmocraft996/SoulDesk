import test from 'node:test';
import assert from 'node:assert/strict';
import { panel,prettyBytes } from './panel-model.js';
const snapshot=()=>({sampledAt:10000,cpu:{percent:11,error:null},memory:{percent:69,error:null},gpu:{percent:19,error:null},details:{cpuLogicalCount:12,memoryTotalBytes:'33285996544',gpuAdapter:'00000000:00000002/0',gpuTemperature:{celsius:55.4,error:null},network:{downloadBytesPerSecond:1024,uploadBytesPerSecond:0,receivedBytes:'15032385536',sentBytes:'1073741824',error:null}}});
test('监控面板保持原四列指标、逻辑处理器和向上取整GiB，温度对应GPU',()=>{
  const result=panel(snapshot(),10000);assert.equal(result.cpu.label,'11%');assert.equal(result.cpu.subtitle,'12 C');assert.equal(result.memory.subtitle,'31 GB');assert.equal(result.gpu.subtitle,'55 ℃');assert.match(result.gpu.detail,/00000000:00000002/);assert.equal(result.network.download,'1 KB');assert.equal(result.network.upload,'0 B');assert.equal(result.network.received,'14.00 GB');
});
test('单项缺失和网络预热互不伪造，旧宿主/过期数据保留缺失原因',()=>{
  const value=snapshot();value.gpu.percent=null;value.gpu.error='GPU正在采样';value.details.network.downloadBytesPerSecond=null;value.details.network.uploadBytesPerSecond=null;value.details.network.error='网络正在采样';
  const result=panel(value,10000);assert.equal(result.gpu.label,'—');assert.equal(result.gpu.subtitle,'— ℃');assert.equal(result.network.download,'—');assert.equal(result.network.received,'14.00 GB');assert.match(result.status,/采样/);
  assert.equal(panel(snapshot(),22000).cpu.label,'—');assert.equal(panel(snapshot(),22000).network.received,'—');
  const old=snapshot();delete old.details;assert.match(panel(old,10000).status,/主程序/);
});
test('累计字节按精确字符串换算，速率与异常值均明确处理',()=>{
  assert.equal(prettyBytes('18446744073709551615',2),'16777216.00 TB');assert.equal(prettyBytes(1536,0),'2 KB');assert.equal(prettyBytes('0',2),'0.00 B');assert.equal(prettyBytes(null,2),'—');assert.equal(prettyBytes(-1,0),'—');assert.equal(prettyBytes('nan',2),'—');
});
