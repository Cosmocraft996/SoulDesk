import test from 'node:test';
import assert from 'node:assert/strict';
const module=await import('./index-model.js').catch(()=>null);
function sample(code){return {code,type:'index',market:'ab',exchange:code==='000001'?'SH':'SZ',name:'指数',price:'12345678901234567890.125',ratio:'0.005',stockStatusInfo:'TRADE',sourceTimeText:''};}
test('三大指数按完整身份读取，缺失价格、错误交易所和非指数不能伪装有效值',()=>{
  assert.ok(module,'指数模型尚未实现');
  assert.equal(module.readIndex(sample('000001'),'000001').price,'12345678901234567890.125');
  for(const patch of [{code:'399001'},{exchange:'SZ'},{type:'stock'},{market:'hk'},{price:'0'},{price:null},{ratio:'NaN'}])assert.throws(()=>module.readIndex({...sample('000001'),...patch},'000001'));
});
test('每个指数独立更新，部分失败保留原行并提示，不把缺报价显示为零',async()=>{
  assert.ok(module);
  let failed=false;
  const model=module.createIndices(async code=>{if(code==='399001')throw new Error('源失败');if(failed&&code==='000001')throw new Error('刷新失败');return {data:sample(code),fetchedAt:10};});
  await model.refresh();assert.equal(model.state.rows.length,3);
  assert.equal(model.state.rows[1].quote,null);assert.equal(model.state.rows[1].error,'源失败');
  assert.equal(model.state.rows[0].quote.price,'12345678901234567890.125');
  failed=true;await model.refresh();assert.equal(model.state.rows[0].quote.price,'12345678901234567890.125');assert.equal(model.state.rows[0].error,'刷新失败');model.dispose();
});
test('重新刷新和卸载后，旧请求不得覆盖当前行',async()=>{
  assert.ok(module);const pending=[];
  const model=module.createIndices(code=>new Promise(resolve=>pending.push({code,resolve})));
  const old=model.refresh(),current=model.refresh();
  pending.slice(3).forEach(item=>item.resolve({data:{...sample(item.code),price:'4000.01'},fetchedAt:20}));await current;
  pending.slice(0,3).forEach(item=>item.resolve({data:sample(item.code),fetchedAt:10}));await old;
  assert.equal(model.state.rows[0].quote.price,'4000.01');
  const abandoned=model.refresh();model.dispose();pending.slice(6).forEach(item=>item.resolve({data:sample(item.code),fetchedAt:30}));await abandoned;
  assert.equal(model.state.rows[0].quote.price,'4000.01');
});
