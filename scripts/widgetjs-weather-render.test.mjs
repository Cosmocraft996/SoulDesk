import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { createSSRApp } from 'vue';
import { renderToString } from 'vue/server-renderer';
import { createServer } from 'vite';

const root='components/widgetjs/weather-source/';
const defaults=()=>Object.fromEntries(Object.entries(JSON.parse(readFileSync('components/widgetjs/weather-large/manifest.json')).settingsSchema.properties).map(([key,schema])=>[key,schema.default]));
async function withApp(run){const vite=await createServer({configFile:resolve(root+'vite.config.mjs'),server:{middlewareMode:true,hmr:false,watch:null},appType:'custom',logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]}});try{const {default:App}=await vite.ssrLoadModule('/App.vue');const icons=await vite.ssrLoadModule('/icons.ts');await run(App,icons);}finally{await vite.close();}}
async function render(App,icons,host,variant='weather-large',seed){let state;const Subject={...App,setup(props,context){state=App.setup(props,context);seed?.(state);return state;}};const app=createSSRApp(Subject,{host,variant,initialSettings:defaults()});app.component('el-tooltip',icons.Tooltip);const html=await renderToString(app);return{state,html};}

test('天气三款原Vue和72张原图保留固定来源，渲染不依赖旧宿主及远程图片',async()=>{
  const source=JSON.parse(readFileSync(root+'SOURCE_FILES.json'));assert.equal(source.revision,'1d5b717b085c82a5b81bfc2f0969bb8518123570');assert.equal(source.files.filter(f=>f.source.startsWith('public/image/')).length,72);
  for(const file of source.files)assert.equal(createHash('sha256').update(readFileSync(root+'original/'+file.source)).digest('hex'),file.sha256,file.source);
  await withApp(async(App,icons)=>{for(const variant of ['weather-small','weather-medium','weather-large']){let calls=0;const {state,html}=await render(App,icons,{preview:true,request(){calls++;throw Error('预览不得访问');},async ready(){}},variant);assert.match(html,/预览 · 不联网/);await state.start();await state.model.refresh();state.dispose();assert.equal(calls,0);assert.equal(state.state.now,null);}});
});

test('实际三个天气模板显示负温度及原观测时间，缺失指标保留缺值，大号独立显示预报错误',async()=>withApp(async(App,icons)=>{
  for(const variant of ['weather-small','weather-medium','weather-large']){
    const {state,html}=await render(App,icons,{preview:false},variant,s=>{s.state.config={configured:true,enabled:true,apiHost:'placeholder.qweatherapi.com'};s.state.now={obsTime:'2026-11-16T07:40+08:00',temp:'-2.5',icon:'100',text:'晴'};s.state.errors.daily='测试预报无权限';});
    assert.match(html,/-2\.5/);assert.match(html,/2026-11-16T07:40\+08:00/);assert.match(html,/集美/);assert.match(html,/测试预报无权限/);assert.doesNotMatch(html,/\/weather\/image\//);assert.doesNotMatch(html,/<el-tooltip/);if(variant!=='weather-large')assert.match(html,/—/);state.dispose();
  }
}));

test('连接设置不回显密钥，失败保留输入，城市设置失败不关闭或覆盖旧城市',async()=>withApp(async(App,icons)=>{
  let failConnection=true,failSettings=true,closed=0;const writes=[];
  const host={preview:false,async request(method,args){if(method==='weather.configure'){writes.push({method,args:structuredClone(args)});if(failConnection)throw Error('连接配置只读');return{configured:true,enabled:false,apiHost:args.apiHost};}if(method==='settings.update'){if(failSettings)throw Error('设置只读');return args;}throw Error(method);}};
  const {state}=await render(App,icons,host);state.dialog.value={showModal(){},close(){closed++;}};state.openSettings();assert.equal(state.apiKey.value,'');state.apiHost.value='placeholder.qweatherapi.com';state.apiKey.value='placeholder_secret';await state.saveConnection();assert.equal(state.apiKey.value,'placeholder_secret');assert.match(state.connectionStatus.value,/连接配置只读/);failConnection=false;await state.saveConnection();assert.equal(state.apiKey.value,'');assert.equal(state.state.config.enabled,false);assert.equal(writes.length,2);
  state.draft.value.cityId='101010100';state.draft.value.cityName='北京';await state.saveSettings();assert.equal(closed,0);assert.equal(state.settings.value.cityName,'集美');assert.equal(state.draft.value.cityName,'北京');assert.match(state.saveError.value,/设置只读/);failSettings=false;await state.saveSettings();assert.equal(closed,1);assert.equal(state.settings.value.cityName,'北京');assert.equal(state.settings.value.apiKey,undefined);state.dispose();
}));

test('初始化读取状态期间离开视图，不再ready、不创建更新循环或请求天气',async()=>withApp(async(App,icons)=>{
  let finish,ready=0;const host={preview:false,request(method){assert.equal(method,'weather.status');return new Promise(resolve=>{finish=resolve;});},ready(){ready++;}};
  const {state}=await render(App,icons,host);const starting=state.start();state.dispose();finish({configured:true,enabled:true,apiHost:'placeholder.qweatherapi.com'});await starting;assert.equal(ready,0);assert.equal(state.state.now,null);
}));

test('密文不可解密时仍完成可恢复界面初始化，让用户清除并重新配置',async()=>withApp(async(App,icons)=>{
  let ready=0;const host={preview:false,async request(method){assert.equal(method,'weather.status');throw Error('凭据不属于当前Windows用户');},async ready(){ready++;}};
  const {state}=await render(App,icons,host);await state.start();state.dispose();
  assert.equal(ready,1);assert.match(state.state.errors.now,/当前Windows用户/);assert.equal(state.state.config.enabled,false);
}));

test('海外字母数字城市标识可保存，跨午夜保留预报原日期，缺失指数项不使模板崩溃',async()=>withApp(async(App,icons)=>{
  const {state,html}=await render(App,icons,{preview:false,async request(method,args){assert.equal(method,'settings.update');return args;}},'weather-large',s=>{
    s.clock.value=Date.parse('2026-11-16T16:30:00Z');s.state.config={configured:true,enabled:true};
    s.state.now={obsTime:'2026-11-17T00:20+08:00',temp:'10',icon:'100',text:'晴'};
    s.state.daily=[{fxDate:'2026-11-16',tempMin:'1',tempMax:'10',iconDay:'100'},{fxDate:'2026-11-17',tempMin:'2',tempMax:'11',iconDay:'100'}];
    s.state.indices=[null,{date:'2026-11-16',text:'旧建议'}];
  });
  assert.equal(html.includes('2026-11-16（已过期）'),true);assert.equal(html.includes('今天'),true);
  assert.equal(html.includes('生活指数日期已过期或未提供'),true);assert.equal(html.includes('三日预报含过期日期'),true);
  state.state.config.enabled=false;let closed=0;state.dialog.value={showModal(){},close(){closed++;}};state.openSettings();
  state.draft.value.cityId='TEST123';state.draft.value.cityName='夹具海外城市';state.draft.value.timezone='America/Los_Angeles';await state.saveSettings();
  assert.equal(closed,1);assert.equal(state.settings.value.cityId,'TEST123');assert.equal(state.settings.value.timezone,'America/Los_Angeles');state.dispose();
}));
