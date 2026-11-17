import{createApp,node,onMounted,onBeforeUnmount,elementPlus}from'./derived/index-Bfkv8pFi.js';
import Search from'./derived/JobView-DmmzGjQr.js';
import Analysis from'./derived/JobAnalysisView-DVV9mC5g.js';
import Knowledge from'./derived/KnowledgeView-Cjfd7KFs.js';
import{createUi}from'./ui.js';
export function mountJob(host,container,{shell=false}={}){
 const ui=createUi(host,shell);
 const app=createApp({setup(){onMounted(()=>ui.connect().catch(ui.client.fail));onBeforeUnmount(ui.dispose);return()=>{
  const page=ui.state.surface?.page?.page,view={search:Search,analysis:Analysis,knowledge:Knowledge}[page];
  return node('div',{class:['job-root',shell?'job-shell':'job-card-root'],onSubmit:event=>event.preventDefault()},[ui.toolbar(),ui.extra(),
   shell?(ui.state.loading?node('p',null,'正在读取本机记录…'):view?node('fieldset',{class:'job-original-page',disabled:ui.locked.value},[node(view)]):node('p',null,'窗口尚未就绪，请关闭后重新打开')):
   node('div',{class:'job-local-card'},[node('h2',null,'AI直聘'),node('p',null,'手动登录 BOSS，按你的求职条件采集与评分。'),node('p',null,'职位分析、知识百科、思维导图与模拟面试。'),node('small',null,'模型由你配置；点击“开始查找”或“生成”后才请求。')])]);};}});
 app.provide('job-ui',ui);app.use(elementPlus);app.mount(container);return{app,ui,dispose:()=>app.unmount()};
}
