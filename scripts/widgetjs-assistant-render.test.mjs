// 使用真实Vue原模板离线渲染，不创建浏览器或原生网页。
import test from'node:test';import assert from'node:assert/strict';import{resolve}from'node:path';
import{createSSRApp}from'vue';import{renderToString}from'vue/server-renderer';import{createServer}from'vite';
import ElementPlus,{ID_INJECTION_KEY,ZINDEX_INJECTION_KEY}from'element-plus';
import{defaultAssistantConfig}from'../components/widgetjs/ai-source/assistant-client.js';
async function withVue(run){const vite=await createServer({configFile:resolve('components/widgetjs/ai-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,ws:false,watch:null},appType:'custom',logLevel:'silent',optimizeDeps:{noDiscovery:true,include:[]}});try{await run(vite);}finally{await vite.close();}}
function app(component,props){return createSSRApp(component,props).use(ElementPlus).provide(ID_INJECTION_KEY,{prefix:81,current:0}).provide(ZINDEX_INJECTION_KEY,{current:0});}
test('原助手保留1/2/4网格、各格选择器及原工具栏，没有Electron标签或远程iframe',()=>withVue(async vite=>{
 const{default:App}=await vite.ssrLoadModule('/Assistant.vue');
 for(const count of[1,2,4]){
  const Subject={...App,setup(p,c){const s=App.setup(p,c);s.ui.state.value.config={...defaultAssistantConfig(),pageCount:count,tipProgress:3};return s;}};
  const html=await renderToString(app(Subject,{shell:true,host:{preview:true,request(){assert.fail('离线渲染不调用宿主');}}}));
  assert.match(html,new RegExp('size-'+count));assert.equal((html.match(/class="native-page"/g)??[]).length,count);assert.equal((html.match(/class="select-trigger"/g)??[]).length,count);
  assert.match(html,/单页布局/);assert.match(html,/双行布局/);assert.match(html,/四格布局/);assert.match(html,/收起助手/);assert.doesNotMatch(html,/<webview|<iframe|configStore/);
 }
}));
test('普通桌面卡展示真实配置与有限入口，预览不打开网页或冒称已登录',()=>withVue(async vite=>{
 const{default:App}=await vite.ssrLoadModule('/Assistant.vue');const html=await renderToString(app(App,{shell:false,host:{preview:true,request(){assert.fail('预览不访问宿主');}}}));
 assert.match(html,/网页 AI 助手/);assert.match(html,/打开助手/);assert.match(html,/后台入口未启用/);assert.match(html,/ChatGPT/);assert.match(html,/预览仅展示控制卡/);assert.doesNotMatch(html,/已登录|<webview|<iframe/);
}));
test('原设置保留左右/四种宽度/代理与快捷键，新增后台和置顶默认关闭',()=>withVue(async vite=>{
 const{default:App}=await vite.ssrLoadModule('/Assistant.vue');const Subject={...App,setup(p,c){const s=App.setup(p,c);s.ui.settingsOpen.value=true;return s;}};
 const html=await renderToString(app(Subject,{shell:false,host:{preview:true}}));
 for(const text of['组件设置','呼出快捷键','左侧','右侧','20%','30%','40%','50%','跟随系统','手动配置','后台入口','置顶','保存设置'])assert.ok(html.includes(text),text);
 assert.equal((html.match(/role="switch" aria-checked="false"/g)??[]).length,2);assert.match(html,/默认关闭/);
}));
test('等待原生完成时不把打开或收起目标状态冒称已显示',()=>withVue(async vite=>{
 const{default:App}=await vite.ssrLoadModule('/Assistant.vue');
 for(const[phase,settled,text]of[['ready',false,'正在显示'],['ready',true,'已打开'],['hidden',false,'正在收起'],['hidden',true,'后台保留']]){
  const Subject={...App,setup(p,c){const s=App.setup(p,c);s.ui.state.value.surface={phase,nativeSettled:settled};return s;}};
  const html=await renderToString(app(Subject,{shell:false,host:{preview:true}}));assert.match(html,new RegExp(text));if(!settled)assert.doesNotMatch(html,/已打开|后台保留/);
 }
}));
