import test from 'node:test';import assert from 'node:assert/strict';
const module=await import('./coin-model.js').catch(()=>null);
const quote=(timestamp,price='12345678901234567890.125')=>({instrument:'BTC-USD',price,reference:'12345678901234567890.12',basis:'open24h',timestamp});
test('指数按原币种和时间校验、精确算涨跌，实际采样绘图不生成随机历史',async()=>{
  assert.ok(module,'虚拟币指数模型尚未实现');let time=100;
  const model=module.createCoin(async()=>({data:quote(time)}));
  await model.select('BTC-USD');assert.equal(model.state.points.length,1);assert.equal(model.state.quote.change.amount,'0.005');
  await model.refresh();assert.equal(model.state.points.length,1);
  time=200;await model.refresh();assert.equal(model.state.points.length,2);
  assert.throws(()=>module.readIndex({...quote(100),instrument:'ETH-USD'},'BTC-USD'));
  model.dispose();
});
test('旧时间或旧币种不能覆盖最新指数，源失败保持旧数据并提示',async()=>{
  assert.ok(module);const pending=[];const model=module.createCoin(instrument=>new Promise((resolve,reject)=>pending.push({instrument,resolve,reject})));
  const old=model.select('BTC-USD'),newer=model.select('ETH-USD');assert.equal(model.state.quote,null);
  pending[1].resolve({data:{...quote(200,'100'),instrument:'ETH-USD'}});await newer;pending[0].resolve({data:quote(100)});await old;
  assert.equal(model.state.quote.instrument,'ETH-USD');
  const back=model.refresh();pending[2].resolve({data:{...quote(100,'99'),instrument:'ETH-USD'}});await back;assert.equal(model.state.quote.price,'100');assert.match(model.state.error,/较旧/);
  const failed=model.refresh();pending[3].reject(new Error('读取失败'));await failed;assert.equal(model.state.quote.price,'100');assert.equal(model.state.error,'读取失败');model.dispose();
});
test('参考基准缺失不冒充零涨跌，UTC+8基准单独保留，旧价格标识明确',()=>{
  assert.ok(module);
  assert.equal(module.readIndex({...quote(100),reference:null,basis:null},'BTC-USD').change,null);
  assert.equal(module.readIndex({...quote(100),basis:'sodUtc8'},'BTC-USD').basis,'sodUtc8');
  assert.equal(module.isOld(quote(100),200000,60000),true);
  assert.equal(module.isOld(quote(100),100,60000),false);
});
test('虚拟币设置只允许原目录和刷新选项，独立代理缺少主机端口时不能保存',()=>{
  assert.ok(module);const base={coin:'BTC-USD',refreshMs:60000,proxyMode:'inherit',proxyHost:'',proxyPort:0};
  assert.deepEqual(module.validateSettings(base),base);
  for(const patch of [{coin:'BTC-USDT'},{refreshMs:3000},{proxyMode:'http'},{proxyMode:'socks5',proxyHost:'name@host',proxyPort:1080}])assert.throws(()=>module.validateSettings({...base,...patch}));
});

test('重建后先恢复真实采样，后台读取中及失败时保留原价格和曲线',async()=>{
  let reject;
  const model=module.createCoin(()=>new Promise((_,failed)=>{reject=failed;}));
  assert.equal(typeof model.restore,'function');
  const cached={data:quote(200,'100.25'),samples:[{timestamp:100,price:'99.5'},{timestamp:200,price:'100.25'}]};
  assert.equal(model.restore(cached,'BTC-USD'),true);
  const refresh=model.select('BTC-USD');
  assert.equal(model.state.loading,true);
  assert.equal(model.state.quote.price,'100.25');
  assert.equal(model.state.points.length,2);
  assert.equal(module.isOld(model.state.quote,200000,60000),true);
  reject(new Error('临时离线'));await refresh;
  assert.equal(model.state.quote.price,'100.25');
  assert.equal(model.state.points.length,2);
  assert.equal(model.state.error,'临时离线');model.dispose();
});

test('缓存的币种、时间顺序和末点必须匹配，不接收拼接或伪造采样',()=>{
  const model=module.createCoin(async()=>({data:quote(200)}));
  assert.equal(typeof model.restore,'function');
  assert.equal(model.restore(null,'BTC-USD'),false);
  const valid={data:quote(200,'100'),samples:[{timestamp:100,price:'99'},{timestamp:200,price:'100'}]};
  for(const patch of [{data:{...valid.data,instrument:'ETH-USD'}},{samples:[{timestamp:200,price:'100'},{timestamp:100,price:'99'}]},{samples:[{timestamp:200,price:'101'}]},{samples:[]}])assert.throws(()=>model.restore({...valid,...patch},'BTC-USD'));
  assert.equal(model.state.quote,null);
  assert.deepEqual(model.state.points,[]);model.dispose();
});

test('主动切换网络口径清除旧展示，旧网络的晚到结果不能回填',async()=>{
  let resolve;
  const model=module.createCoin(()=>new Promise(done=>{resolve=done;}));
  model.restore({data:quote(200,'100'),samples:[{timestamp:200,price:'100'}]},'BTC-USD');
  const old=model.refresh();
  assert.equal(typeof model.reset,'function');
  model.reset();resolve({data:quote(300,'999')});await old;
  assert.equal(model.state.quote,null);
  assert.deepEqual(model.state.points,[]);
  assert.equal(model.state.instrument,null);model.dispose();
});
