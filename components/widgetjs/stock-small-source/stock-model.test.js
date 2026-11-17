import test from 'node:test';
import assert from 'node:assert/strict';
const module=await import('./stock-model.js').catch(()=>null);
const selected={code:'01810',marketId:116,name:'小米集团'};
const quote=()=>({code:'01810',marketId:116,name:'小米集团',decimal:3,previousClose:'12345678901234567890.125',times:['2026-11-14 10:00'],values:['12345678901234567890.135'],sourceTime:1794657600,sourceStatus:1});
test('股票小组件按市场与代码匹配，精确算涨跌；空图不生成随机价格',()=>{
  assert.ok(module,'股票分时模型尚未实现');
  const value=module.readStock(quote(),selected);assert.equal(value.price,'12345678901234567890.135');assert.equal(value.change.amount,'0.01');
  assert.throws(()=>module.readStock({...quote(),marketId:1},selected));
  const empty=module.readStock({...quote(),times:[],values:[]},selected);assert.equal(empty.price,null);assert.equal(empty.change,null);
  assert.equal(module.readStock({...quote(),previousClose:null},selected).change,null);
});
test('换证券先撤下原报价，迟到响应/失败不能把旧证券显示为新证券',async()=>{
  assert.ok(module);const pending=[];
  const model=module.createStock(selection=>new Promise((resolve,reject)=>pending.push({selection,resolve,reject})));
  const first=model.select(selected);pending[0].resolve({data:quote()});await first;
  const next={code:'00700',marketId:116,name:'腾讯控股'};
  const changed=model.select(next);assert.equal(model.state.quote,null);pending[1].reject(new Error('源失败'));await changed;
  assert.equal(model.state.quote,null);assert.equal(model.state.error,'源失败');
  const old=model.select(selected),newer=model.select(next);pending[3].resolve({data:{...quote(),code:'00700',name:'腾讯控股'}});await newer;pending[2].resolve({data:quote()});await old;
  assert.equal(model.state.quote.code,'00700');model.dispose();
});
test('搜索结果保留完整市场身份、限制非法证券，设置与取消不改变已保存证券',()=>{
  assert.ok(module);
  assert.equal(module.validateSelection(selected).marketId,116);
  for(const value of [{...selected,marketId:999},{...selected,code:'A&url=x'},{...selected,name:''}])assert.throws(()=>module.validateSelection(value));
});
