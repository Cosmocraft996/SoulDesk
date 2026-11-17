import test from 'node:test';
import assert from 'node:assert/strict';
const module = await import('./weather-model.js').catch(() => null);
const now = { obsTime:'2026-11-16T07:40+08:00', temp:'-2.5', icon:'100', text:'晴' };

test('未配置、已关闭和隐藏预览不读取天气或搜索，不用零温度代替缺值', async () => {
  assert.ok(module, '天气模型尚未实现');
  for (const preview of [false, true]) {
    let calls=0; const model=module.createWeatherModel(async()=>{calls++;throw Error('不应请求');},{preview,getSettings:()=>({cityId:'101010100'})});
    await model.refresh(); await model.lookup('北京');
    assert.equal(calls,0); assert.equal(model.state.now,null);model.dispose();
  }
  assert.equal(module.temperature(null,'C'),null);
  assert.equal(module.temperature('-2.5','F'),'27.5');
});

test('大号天气局部预报失败保留实时值，观测时间不替换成读取时间', async () => {
  assert.ok(module);const model=module.createWeatherModel(async(method,args)=>{
    if(method==='weather.status')return{configured:true,enabled:true,apiHost:'example.qweatherapi.com'};
    if(args.kind==='now')return{data:{code:'200',now},fetchedAt:999999};
    if(args.kind==='daily')throw Error('预报无权限');
    return{data:{code:'200',daily:[{text:'天气建议'}]}};
  },{large:true,getSettings:()=>({cityId:'101010100'})});
  await model.readStatus();await model.refresh();
  assert.equal(model.state.now.temp,'-2.5');assert.equal(model.state.now.obsTime,now.obsTime);
  assert.match(model.state.errors.daily,/预报无权限/);assert.equal(model.state.daily,null);
  assert.equal(model.state.indices[0].text,'天气建议');model.dispose();
});

test('切城市或销毁后，旧请求不能回写；刷新失败保留原天气并提示', async () => {
  assert.ok(module);let cityId='101010100',finish,fail=false;
  const model=module.createWeatherModel(async(method)=>{if(method==='weather.status')return{configured:true,enabled:true};if(fail)throw Error('离线');return await new Promise(resolve=>{finish=resolve;});},{getSettings:()=>({cityId})});
  await model.readStatus();const old=model.refresh();cityId='101230206';model.reset();finish({data:{code:'200',now}});await old;
  assert.equal(model.state.now,null);
  const fresh=model.refresh();finish({data:{code:'200',now}});await fresh;fail=true;await model.refresh();
  assert.equal(model.state.now.temp,'-2.5');assert.match(model.state.errors.now,/离线/);
  fail=false;const late=model.refresh();model.dispose();finish({data:{code:'200',now:{...now,temp:'99'}}});await late;
  assert.equal(model.state.now.temp,'-2.5');
});

test('预报按来源日期和城市时区标今天/明天，午夜后旧数组不把昨日冒充今天',()=>{
  assert.ok(module);assert.equal(typeof module.forecastDayText,'function');
  const time=Date.parse('2026-11-16T16:30:00Z');
  assert.equal(module.forecastDayText('2026-11-16','Asia/Shanghai',time),'2026-11-16（已过期）');
  assert.equal(module.forecastDayText('2026-11-17','Asia/Shanghai',time),'今天');
  assert.equal(module.forecastDayText('2026-11-18','Asia/Shanghai',time),'明天');
  assert.equal(module.forecastDayText('2026-11-16','America/Los_Angeles',time),'今天');
  assert.equal(module.periodIsPast(undefined,'Asia/Shanghai',time),true);
});
