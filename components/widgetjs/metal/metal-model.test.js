import test from 'node:test';
import assert from 'node:assert/strict';
const module=await import('./metal-model.js').catch(()=>null);
const now=Date.parse('2026-11-14T12:00:00Z');
const sample=()=>['XAUUSD','XAGUSD','XCUUSD','XPTUSD'].map((code,index)=>({code,name:['Gold','Silver','Copper','Platinum'][index],price:'12345678901234567890.1234567',changePercent:'-0.005',updatedAt:'2026-11-14T12:00:00Z',stale:false,marketState:'open'}));
test('贵金属显示保留十进制精度，金额与涨跌百分比按相同规则舍入',()=>{
  assert.ok(module,'贵金属模型尚未实现');
  assert.equal(module.formatDecimal('12345678901234567890.125'), '12,345,678,901,234,567,890.13');
  assert.equal(module.formatDecimal('-0.005'),'-0.01');
  assert.equal(module.formatDecimal('-0.004'),'0.00');
  assert.equal(module.signOf('0.000'),0);assert.equal(module.signOf('-0.1'),-1);
  for(const value of ['NaN','1e30','',null,1])assert.throws(()=>module.formatDecimal(value));
});
test('数据身份和四种报价必须完整；过期、未来时间和源标记都显示旧数据',()=>{
  assert.ok(module);
  const value=module.readMetals(sample());assert.equal(value[0].price,'12345678901234567890.1234567');
  assert.equal(module.quoteIsOld(value[0],now),false);
  assert.equal(module.quoteIsOld(value[0],now+91000),true);
  assert.equal(module.quoteIsOld({...value[0],stale:true},now),true);
  assert.equal(module.quoteIsOld({...value[0],quoteAgeSeconds:120},now),true);
  assert.equal(module.quoteIsOld({...value[0],updatedAt:'2026-11-14T13:00:00Z'},now),true);
  const wrong=sample();wrong[0].code='BTCUSD';assert.throws(()=>module.readMetals(wrong));
  const zero=sample();zero[0].price='0';assert.throws(()=>module.readMetals(zero));
  assert.throws(()=>module.readMetals(sample().slice(0,3)));
  const ambiguous=sample();ambiguous[0].updatedAt='2026-11-14T12:00:00';assert.throws(()=>module.readMetals(ambiguous));
});
test('刷新失败保留旧行情，旧请求及时间倒退响应不能覆盖新报价',async()=>{
  assert.ok(module);const pending=[];
  const model=module.createMetals(()=>new Promise((resolve,reject)=>pending.push({resolve,reject})));
  const first=model.refresh(),second=model.refresh();const newest=sample();newest[0].price='3000.01';
  pending[1].resolve({data:newest,fetchedAt:now});await second;
  pending[0].resolve({data:sample(),fetchedAt:now-1});await first;
  assert.equal(model.state.quotes[0].price,'3000.01');
  const failed=model.refresh();pending[2].reject(new Error('读取失败'));await failed;
  assert.equal(model.state.quotes[0].price,'3000.01');assert.equal(model.state.error,'读取失败');
  const backwards=model.refresh();const old=sample().map(item=>({...item,updatedAt:'2026-11-14T11:00:00Z'}));pending[3].resolve({data:old,fetchedAt:now+1});await backwards;
  assert.equal(model.state.quotes[0].price,'3000.01');assert.match(model.state.error,/较旧/);
  model.dispose();
});
