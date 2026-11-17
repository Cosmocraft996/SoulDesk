import test from 'node:test';
import assert from 'node:assert/strict';
const module=await import('./stock-model.js').catch(()=>null);
const stock=(marketId=1)=>({code:'000001',marketId,name:'夹具证券',holdingPrice:'0.1',holdingShares:'0.3'});
const quote=(marketId=1,price='0.2',sourceTime=1794557495)=>({code:'000001',marketId,name:'夹具证券',price,previousClose:'0.15',sourceTime,decimal:3,identityBasis:'request-secid'});

test('手工持仓精确计算，不用浮点误差或缺值冒充零收益',()=>{
  assert.ok(module,'完整股票持仓模型未实现');
  const value=module.holdingMetrics(stock(),quote());assert.equal(value.returnAmount,'0.03');assert.equal(value.returnRate,'100.00');
  assert.equal(module.holdingMetrics({...stock(),holdingPrice:'0'},quote()).returnRate,null);
  assert.equal(module.holdingMetrics({...stock(),holdingShares:''},quote()).returnAmount,null);
  assert.equal(module.holdingMetrics(stock(),null).returnAmount,null);
  const precise=module.holdingMetrics({...stock(),holdingPrice:'12345678901234567890.115',holdingShares:'3'},quote(1,'12345678901234567890.125'));
  assert.equal(precise.returnAmount,'0.03');assert.equal(precise.change.amount,'12345678901234567889.975');
});

test('自选身份包含市场，同代码不同市场可独立保存，非法持仓和重复行拒绝',()=>{
  assert.ok(module);const data=module.normalizePortfolio({version:1,stocks:[stock(0),stock(1)]});assert.equal(data.stocks.length,2);assert.notEqual(module.stockKey(data.stocks[0]),module.stockKey(data.stocks[1]));
  for(const stocks of [[stock(),stock()],[{...stock(),holdingPrice:'-1'}],[{...stock(),holdingPrice:0.1}],[{...stock(),holdingShares:'NaN'}]])assert.throws(()=>module.normalizePortfolio({version:1,stocks}));
  assert.throws(()=>module.normalizePortfolio({version:2,stocks:[]}));
  assert.throws(()=>module.readQuote({...quote(),decimal:2,price:'0.1234567890123456789012345678'},stock()),/精度/);
  assert.throws(()=>module.readQuote({...quote(),decimal:2,previousClose:'0.1234567890123456789012345678'},stock()),/精度/);
});

test('部分刷新失败保留对应旧报价与源时间，身份错误拒绝且旧报价不回退',async()=>{
  assert.ok(module);let fail=false,wrong=false,older=false;
  const model=module.createPortfolio(async s=>{if(fail&&s.marketId===0)throw Error('测试离线');return{data:quote(wrong?116:s.marketId,fail?'0.3':'0.2',older?1794557000:1794557495),fetchedAt:99};},async value=>value,{version:1,stocks:[stock(0),stock(1)]});
  await model.refresh();fail=true;await model.refresh();assert.equal(model.state.quotes['0:000001'].data.price,'0.2');assert.match(model.state.quotes['0:000001'].error,/离线/);assert.equal(model.state.quotes['1:000001'].data.price,'0.3');
  fail=false;wrong=true;await model.refresh();assert.equal(model.state.quotes['1:000001'].data.price,'0.3');assert.match(model.state.quotes['1:000001'].error,/身份/);
  wrong=false;older=true;await model.refresh();assert.match(model.state.quotes['1:000001'].error,/较旧/);assert.equal(model.state.quotes['1:000001'].data.sourceTime,1794557495);model.dispose();
});

test('保存失败不改旧列表，晚回报价不能复活已删除行，销毁后不回写',async()=>{
  assert.ok(module);let reject=true,finish;
  const model=module.createPortfolio(()=>new Promise(resolve=>{finish=resolve;}),async value=>{if(reject)throw Error('数据库只读');return value;},{version:1,stocks:[stock()]});
  await assert.rejects(model.save({version:1,stocks:[]}),/只读/);assert.equal(model.state.portfolio.stocks.length,1);
  const pending=model.refresh();reject=false;await model.save({version:1,stocks:[]});finish({data:quote(),fetchedAt:1});await pending;assert.equal(model.state.portfolio.stocks.length,0);assert.deepEqual(model.state.quotes,{});
  await model.save({version:1,stocks:[stock()]});const late=model.refresh();model.dispose();finish({data:quote(),fetchedAt:1});await late;assert.deepEqual(model.state.quotes,{});
});

test('预览模型不读取或保存实例持仓',async()=>{
  assert.ok(module);let calls=0;const forbidden=async()=>{calls++;throw Error('预览不访问');};
  const model=module.createPortfolio(forbidden,forbidden,{version:1,stocks:[stock()]},{preview:true});await model.refresh();await assert.rejects(model.save({version:1,stocks:[]}),/不能保存/);assert.equal(calls,0);model.dispose();
});
