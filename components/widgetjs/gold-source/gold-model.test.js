import test from 'node:test';
import assert from 'node:assert/strict';
const module=await import('./gold-model.js').catch(()=>null);
const quote=()=>({instrument:'Au99.99',code:'AU9999',latestPrice:'700.12',yesterdayClose:'700',min:'699',max:'701',timestamp:Date.parse('2026-11-14T02:00:00Z')});
const trends=()=>({code:'AU9999',times:['2026-11-14 09:59','2026-11-14 10:00'],values:['700.11','700.12']});
test('黄金涨跌与百分比使用精确十进制，缺少昨收时不冒充零涨跌',()=>{
  assert.ok(module,'黄金模型尚未实现');
  const change=module.goldChange({...quote(),latestPrice:'12345678901234567890.02',yesterdayClose:'12345678901234567890.01'});
  assert.equal(change.amount,'0.01');assert.equal(change.percent,'0.00');assert.equal(change.sign,1);
  assert.equal(module.goldChange({...quote(),yesterdayClose:null}),null);
  assert.equal(module.goldChange({...quote(),latestPrice:'699.965',yesterdayClose:'700'}).percent,'-0.01');
});
test('分时图在大价格小波动和单点时仍有有效坐标，不把空图伪装为零价',()=>{
  assert.ok(module);
  const geometry=module.chartGeometry(['12345678901234567890.01','12345678901234567890.02'],'12345678901234567890.02',100,80);
  assert.equal(geometry.points[0].y,76);assert.equal(geometry.points[1].y,4);
  assert.ok(Number.isFinite(module.chartGeometry(['700'],'700',100,80).points[0].x));
  assert.deepEqual(module.chartGeometry([],'700',100,80).points,[]);
});
test('黄金原合约和时间必须明确，分时图日期与报价不符时不混合展示',()=>{
  assert.ok(module);
  assert.equal(module.readGold(quote()).latestPrice,'700.12');
  assert.throws(()=>module.readGold({...quote(),code:'AU9995'}));
  assert.throws(()=>module.readGold({...quote(),latestPrice:'0'}));
  assert.equal(module.chartMatchesQuote(module.readTrends(trends()),quote()),true);
  assert.equal(module.chartMatchesQuote(module.readTrends({...trends(),times:['2026-11-13 10:00'],values:['700']}),quote()),false);
});
test('图源失败不丢有效报价，旧刷新响应不能覆盖新一轮数据',async()=>{
  assert.ok(module);const quotePending=[],chartPending=[];
  const model=module.createGold(()=>new Promise((resolve,reject)=>quotePending.push({resolve,reject})),()=>new Promise((resolve,reject)=>chartPending.push({resolve,reject})));
  const first=model.refresh(),second=model.refresh();
  await Promise.resolve();
  quotePending[1].resolve({data:{...quote(),latestPrice:'701'}});chartPending[1].reject(new Error('分时源失败'));await second;
  quotePending[0].resolve({data:quote()});chartPending[0].resolve({data:trends()});await first;
  assert.equal(model.state.quote.latestPrice,'701');assert.equal(model.state.chartError,'分时源失败');
  const failed=model.refresh();await Promise.resolve();quotePending[2].reject(new Error('报价源失败'));chartPending[2].resolve({data:trends()});await failed;
  assert.equal(model.state.quote.latestPrice,'701');assert.equal(model.state.error,'报价源失败');model.dispose();
});
