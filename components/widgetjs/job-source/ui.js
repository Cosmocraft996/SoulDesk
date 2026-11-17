// 保留原页面渲染与主题，替换持久化、原生网页和模型调用。
import{ref,reactive,computed,inject,node,onMounted,onBeforeUnmount,watch,nextTick}from'./derived/index-Bfkv8pFi.js';
import{originalMarkdown}from'./derived/KnowledgeView-Cjfd7KFs.js';
import createDOMPurify from'dompurify';
import{mindmapData,mountMindmap}from'../ai-source/graphics.js';
import{createJobClient,defaultConfig,kinds}from'./client.js';
const clone=value=>JSON.parse(JSON.stringify(value));
const phaseText={waiting_page:'采集中',ready_to_rate:'准备评分',waiting_model:'分析职位中',completed:'已完成',stopped:'已停止',interrupted:'已中断',failed:'失败',running:'生成中',cancelled:'已停止'};
const activeRun=run=>['waiting_page','ready_to_rate','waiting_model'].includes(run?.phase);
export function createUi(host,shell){
 const state=reactive({}),client=createJobClient(host,{shell,changed:value=>Object.assign(state,value)});Object.assign(state,clone(client.state));
 const draft=reactive(defaultConfig()),modelDraft=reactive({url:'',model:'',apiKey:'',enabled:false}),modelDirty=ref(false),notice=ref('');
 const flags=reactive({sidebar:true,logs:false,settings:false}),logs=ref([]),contextDraft=reactive({knowledge:'',jobName:'',industry:'',extra:''});
 const filter=reactive({searchKeyword:'',score:5,page:1,pageSize:20});
 let configBase=0,modelBase=0,dirty=false,applying=false,querySeq=0,timer,disposed=false,overlayQueue=Promise.resolve(),bossElement,resizeObserver,layoutTimer,lastLayout='',lastRun='',queryRun='',polling=false;
 const active=computed(()=>activeRun(state.run)),locked=computed(()=>host.preview||state.loading||state.busy||state.pending);
 const running=kind=>state.knowledge[kind]?.job?.phase==='running';
 const report=error=>client.fail(error);
 const perform=fn=>Promise.resolve().then(fn).catch(error=>{report(error);return false;});
 const ui={host,shell,state,client,draft,modelDraft,flags,filter,notice,active,locked,contextDraft,
  contextChanged:()=>JSON.stringify(contextDraft)!==JSON.stringify(state.surface?.page?.context??contextDraft),
  link:url=>{const keys={'https://platform.deepseek.com/usage':'deepseek','https://help.aliyun.com/zh/model-studio/getting-started/first-api-call-to-qwen':'qwen','https://xinghuo.xfyun.cn/sparkapi':'spark','https://cloud.tencent.com/document/product/1772/115970':'tencent'};return perform(()=>host.preview?false:host.request('jobs.help',{topic:keys[url]??'invalid'}));},
 };
 function log(message,level='info'){const row={timestamp:Date.now(),message,level};logs.value=[...logs.value.slice(-199),row];}
 watch(()=>draft,()=>{if(!applying)dirty=true;},{deep:true,flush:'sync'});
 watch(()=>modelDraft,()=>{if(!applying)modelDirty.value=true;},{deep:true,flush:'sync'});
 function syncDraft(){if(!dirty){applying=true;Object.assign(draft,clone(state.config));configBase=state.revision;applying=false;}if(!modelDirty.value){applying=true;Object.assign(modelDraft,{url:state.model.apiBaseUrl,model:state.model.model,apiKey:'',enabled:state.model.enabled});modelBase=state.model.revision;applying=false;}
  if(state.surface?.page?.page==='knowledge'&&!contextDraft.knowledge)Object.assign(contextDraft,clone(state.surface.page.context));
  const key=JSON.stringify(state.run);if(key!==lastRun&&state.run){lastRun=key;log(`${phaseText[state.run.phase]??state.run.phase} · 第${state.run.page}页，已采集${state.run.seen}项，已评分${state.run.rated}项`,state.run.phase==='failed'?'error':'info');}
 }
 async function save(){if(!await client.configure(clone(draft),configBase))return false;dirty=false;syncDraft();notice.value='求职设置已保存';return true;}
 async function query(){const seq=++querySeq;const result=await client.query(clone(filter));return seq===querySeq&&result;}
 async function waitOverlay(value){
  if(!shell||state.surface?.page?.page!=='search'||host.preview)return true;
  if(state.surface?.nativeSettled&&state.surface.overlay===value)return true;
  let result=await client.surface('overlay',{active:value});
  for(let i=0;i<100;i++){
   if(disposed)return false;if(result?.fault)throw Error(result.fault);
   if(result?.nativeSettled&&result.overlay===value)return true;
   await new Promise(resolve=>setTimeout(resolve,50));result=await client.surface('info');
  }
  throw Error('HOST_TIMEOUT: 网页显隐尚未确认，请重试');
 }
 function desiredOverlay(){return flags.sidebar||flags.logs||flags.settings||active.value;}
 function flag(name,value){overlayQueue=overlayQueue.catch(()=>{}).then(async()=>{if(disposed)return false;if(value&&!await waitOverlay(true))return false;flags[name]=value;if(!desiredOverlay())await waitOverlay(false);return true;}).catch(report);return overlayQueue;}
 function linked(name){return computed({get:()=>flags[name],set:value=>void flag(name,value)});}
 async function start(jobId){if(locked.value)return false;if(!jobId&&(!draft.search.searchKeyword.trim()||!state.model.enabled)){report('请填写职位关键词，保存并启用自己的模型连接');return false;}
  if(dirty&&!await save())return false;
  if(await client.start(jobId)){flags.sidebar=false;notice.value='任务已启动；模型调用可能产生费用';await waitOverlay(desiredOverlay());return true;}return false;
 }
 async function generate(kind,text){if(locked.value||running(kind))return false;if(ui.contextChanged()){await client.open({page:'knowledge',context:clone(contextDraft)});notice.value='已打开新知识窗口，请在新窗口点击生成';return false;}return client.generate(kind,text);}
 async function generateBoth(){if(ui.contextChanged())return generate('wiki');if(await generate('wiki'))await generate('mindmap');}
 function chat(kind){return{
  messages:computed(()=>{const d=state.knowledge[kind],messages=d?.messages.filter(m=>!m.hidden).map((m,index)=>({...m,id:(m.requestId??'greeting')+':'+m.role+':'+index}))??[];if(kind==='interview'&&d?.job?.partial&&!['running','completed'].includes(d.job.phase))messages.push({id:d.job.id+':partial',role:'assistant',content:`未完成内容（${phaseText[d.job.phase]}）\n\n${d.job.partial}`});return messages;}),
  latestMessageContent:computed(()=>{const d=state.knowledge[kind];return d?.job&&d.job.phase!=='completed'&&d.job.partial?d.job.partial:d?.latestText??'';}),
  streaming:computed(()=>running(kind)),
  sendWithStream:async(text,hidden,done)=>{if(await generate(kind,kind==='interview'?text:undefined))done?.();},
  clear:()=>client.clearKnowledge(kind),updateChat:()=>generateBoth(),
 };}
 async function copyText(text){if(host.preview)return false;await navigator.clipboard.writeText(text??'');notice.value='已复制到剪贴板';return true;}
 async function layout(){if(disposed||!bossElement||!shell)return;const r=bossElement.getBoundingClientRect();const rect={x:Math.max(0,r.x),y:Math.max(0,r.y),width:r.width,height:r.height};if(rect.width<160||rect.height<160)return;const signature=JSON.stringify(rect);if(lastLayout===signature)return;await client.surface('layout',rect);lastLayout=signature;}
 ui.setup=(name,p,ctx)=>{
  const model=(key,prop='modelValue')=>computed({get:()=>p[prop],set:value=>ctx.emit?.('update:'+prop,value)});
  switch(name){
   case'links':return ui.link;
   case'settingsExtras':return ui.settingsExtras;
   case'label':return()=>ui.contextChanged()?'打开新知识':'生成';
   case'JobView':return{t:ref(),n:linked('sidebar'),o:{},s:active,u:computed(()=>phaseText[state.run?.phase]??'采集中'),f:computed(()=>draft.search),w:linked('logs'),m:{stop:()=>client.stop()},i:()=>perform(()=>start())};
   case'JobSearchConfig':return{n:ctx.emit,o:linked('sidebar'),s:computed(()=>draft.search),u:computed(()=>({isConfigured:state.model.enabled})),f:linked('settings'),w:{},m:linked('logs'),i:()=>client.open({page:'analysis'}),y:()=>flag('sidebar',false),v:()=>flag('sidebar',true),_:()=>perform(()=>start()),E:()=>perform(()=>host.preview?false:host.request('jobs.help',{topic:'tutorial'}))};
   case'SettingsDialog':return{t:model('t')};
   case'AiConfigForm':{const selected=ref();return{t:{},n:ref(),o:ref(),s:selected,u:modelDraft,w:{url:[{required:true,message:'请输入请求地址'}],model:[{required:true,message:'请输入模型名称'}]},
    m:()=>perform(async()=>{if(await client.model(clone(modelDraft),modelBase)){modelDraft.apiKey='';modelDirty.value=false;syncDraft();notice.value='模型连接已保存；不会自动生成内容';}}),
    i:provider=>{modelDraft.url=provider.apiUrl.replace(/\/chat\/completions$/,'');modelDraft.model=provider.chatModel;selected.value=provider;},
   };}
   case'LogPanel':return{t:ref(),n:{clear:()=>logs.value=[]},o:ref(500),s:logs,u:linked('logs'),f:()=>logs.value=[],w:()=>flag('logs',false)};
   case'JobAnalysisView':{
    const a=computed({get:()=>filter.score,set:value=>{filter.score=value;filter.page=1;void query();}}),n=computed({get:()=>filter.searchKeyword,set:value=>{filter.searchKeyword=value;filter.page=1;void query();}});
    return{e:ref(Math.max(480,globalThis.innerHeight??760)),t:computed(()=>[{keyword:'',count:state.query?.keywords?.reduce((n,v)=>n+v.count,0)??0},...(state.query?.keywords??[]).map(v=>({keyword:v.name,count:v.count}))]),a,n,i:computed(()=>state.query?.jobs??[]),o:query,s:computed(()=>stat('skills')),l:computed(()=>stat('skills').slice(30)),u:computed(()=>stat('industries')),f:()=>perform(async()=>{if(await client.clear())await query();}),h:value=>client.open({page:'knowledge',context:{knowledge:typeof value==='string'?value:value.name,jobName:'',industry:'',extra:''}})};
   }
   case'PaginationScrollbar':return{e:p,t:ref(),a:ref(),n:ref(1),i:computed(()=>p.data.length),o:computed(()=>p.data)};
   case'JobCard':return{e:p,t:computed(()=>p.job.analysis?.score??0),a:computed(()=>p.job.analysisStale?'旧条件评语：'+(p.job.previousAnalysis?.comment??''):(p.job.analysis?.comment??'未评分')),n:()=>client.open({page:'detail',jobId:p.job.encryptJobId}),i:computed(()=>draft.search),o:computed(()=>active.value&&state.run?.target===p.job.encryptJobId),s:()=>perform(()=>start(p.job.encryptJobId))};
   case'KnowledgePieChart':return{e:p,t:ref([]),a:computed(()=>p.data),n:value=>client.open({page:'knowledge',context:{knowledge:value.name,jobName:'',industry:'',extra:''}})};
   case'KnowledgeView':return{t:ref(),n:ref(true),r:computed(()=>running('wiki')),s:computed(()=>running('mindmap')),i:{},o:computed(()=>running('wiki')||running('mindmap')),l:Object.assign(contextDraft,{fullFill:()=>''}),h:chat('wiki'),f:chat('mindmap'),m:{knowledge:[{required:true,message:'请输入知识点/技能'}]},b:()=>perform(generateBoth)};
   case'InterviewSection':{const n=ref('');return{t:p,n,r:computed(()=>p.context.knowledge),s:ref(false),i:()=>{},o:chat('interview'),l:()=>perform(async()=>{if(n.value.trim()&&await generate('interview',n.value)){n.value='';}}),h:()=>client.clearKnowledge('interview')};}
   case'ChatBox':return{t:ref(),n:null};
   case'ChatInput':return{n:ctx.emit,r:computed({get:()=>p.modelValue,set:value=>ctx.emit('update:modelValue',value)}),s:event=>{if(event.shiftKey&&event.key==='Enter'){event.preventDefault();ctx.emit('submit',p.modelValue);}}};
   case'MarkdownSection':{const s=ref();return{n:p,r:ctx.emit,s,i:()=>perform(()=>copyText(s.value?.innerText??'')),o:()=>perform(()=>copyText(p.markdown))};}
   case'MindMapSection':{const o=ref(),selected=ref('');return{n:p,r:ctx.emit,s:ref(),i:ref(),o,l:ref(false),h:'',f:value=>{selected.value=value.text;o.value?.show(value.x,value.y);},m:()=>perform(()=>copyText(selected.value)),b:()=>client.open({page:'knowledge',context:{knowledge:selected.value,jobName:contextDraft.jobName,industry:contextDraft.industry,extra:p.knowledge}})};}
   default:throw Error('未知原组件接线：'+name);
  }
 };
 ui.provider=ref();
 function stat(key){return(state.query?.[key]??[]).map(v=>({name:v.name,value:v.count}));}
 ui.render=(name,p,ctx)=>{
  if(name==='WindowTitleBar')return()=>null;
  if(name==='Window')return()=>node('div',{class:'window-body'},ctx.slots?.default?.());
  if(name==='BossWebView'){
   const target=ref();onMounted(()=>{bossElement=target.value;if(typeof ResizeObserver!=='undefined'){resizeObserver=new ResizeObserver(()=>{clearTimeout(layoutTimer);layoutTimer=setTimeout(()=>perform(layout),60);});resizeObserver.observe(target.value);}void perform(layout);});onBeforeUnmount(()=>{bossElement=undefined;resizeObserver?.disconnect();clearTimeout(layoutTimer);});
   return()=>node('div',{ref:target,class:'boss-placeholder','aria-label':'BOSS手动登录与搜索网页'});
  }
  if(name==='Markdown'){
   const purify=createDOMPurify(window),html=computed(()=>purify.sanitize(originalMarkdown(p.markdown??''),{ALLOWED_TAGS:['p','br','strong','em','code','pre','span','del','mark','sup','sub','ins','table','thead','tbody','tr','td','th','ul','ol','li','blockquote','h1','h2','h3','h4','h5','h6','hr'],ALLOWED_ATTR:[]}));
   return()=>node('div',{class:'markdown-body',innerHTML:html.value});
  }
  if(name==='ContextMenu'){
   const visible=ref(false),x=ref(0),y=ref(0),hide=()=>visible.value=false;ctx.expose?.({show:(left,top)=>{x.value=Math.min(left,innerWidth-130);y.value=Math.min(top,innerHeight-90);visible.value=true;}});
   onMounted(()=>window.addEventListener('pointerdown',hide));onBeforeUnmount(()=>window.removeEventListener('pointerdown',hide));
   return()=>visible.value?node('div',{class:'job-context-menu',style:{left:x.value+'px',top:y.value+'px'},onPointerdown:event=>event.stopPropagation()},[button('复制',()=>{ctx.emit('copy');hide();}),button('查看',()=>{ctx.emit('view');hide();})]):null;
  }
  if(name==='MarkdownMap'){
   const svg=ref(),error=ref('');let map,sequence=0,debounce;
   async function draw(){const seq=++sequence;try{if(!p.markdown){map?.destroy();map=null;svg.value?.replaceChildren();error.value='点击生成后显示思维导图';return;}const root=await mindmapData(p.markdown);if(disposed||seq!==sequence)return;map??=await mountMindmap(svg.value,root);if(disposed||seq!==sequence){map?.destroy();map=null;return;}await map.setData(root);if(seq===sequence){await map.fit();error.value='';}}catch(e){if(seq===sequence)error.value=String(e.message??e);}}
   onMounted(()=>perform(draw));watch(()=>p.markdown,()=>{clearTimeout(debounce);debounce=setTimeout(draw,200);});onBeforeUnmount(()=>{sequence++;clearTimeout(debounce);map?.destroy();});
   ctx.expose?.({zoomIn:()=>map?.rescale(1.2),zoomOut:()=>map?.rescale(.8),fit:()=>map?.fit()});
   return()=>node('div',{class:'job-map'},[error.value?node('p',null,error.value):null,node('svg',{ref:svg,onContextmenu:event=>{const target=event.target.closest?.('foreignObject')??event.target;if(target?.textContent?.trim()){event.preventDefault();ctx.emit('nodeClick',{text:target.textContent.trim(),x:event.clientX,y:event.clientY});}}})]);
  }
  throw Error('未知原组件渲染：'+name);
 };
 function button(label,run,disabled=locked.value){return node('button',{type:'button',disabled,onClick:()=>perform(run)},label);}
 ui.toolbar=()=>node('div',{class:'job-host-toolbar'},[
  !shell?button('打开AI直聘',()=>client.open({page:'search'})):null,
  button('职位分析',()=>client.open({page:'analysis'})),
  state.surface?.page?.page==='search'?button(flags.sidebar?'登录页面':'求职设置',()=>flag('sidebar',!flags.sidebar)):null,
  active.value?button('停止',()=>client.stop(),host.preview||state.busy):null,
  state.surface?.page?.page==='search'?button('保存求职设置',save):null,
  state.surface?.page?.page==='analysis'?node('span',{class:'job-pagination'},[button('上一页',async()=>{filter.page--;await query();},locked.value||filter.page<=1),node('span',null,`第${filter.page}页 / ${state.query?.total??0}项`),button('下一页',async()=>{filter.page++;await query();},locked.value||!state.query?.hasNext)]):null,
  state.pending?button('重试原请求',async()=>{if(await client.retry()){if(state.lastCompleted==='jobs.configure')dirty=false;if(['jobs.model.configure','jobs.model.clear'].includes(state.lastCompleted)){modelDraft.apiKey='';modelDirty.value=false;}syncDraft();}},state.busy):null,
  node('span',{class:'job-status'},state.run?`${phaseText[state.run.phase]} · 采集${state.run.seen} / 评分${state.run.rated}`:'尚未开始'),
 ]);
 ui.extra=()=>node('div',{class:'job-host-status'},[
  node('p',{role:'status'},state.error||state.pollError||state.execution?.error||state.presentation?.nativeError||state.surface?.fault||notice.value),
  state.surface?.page?.page==='knowledge'&&ui.contextChanged()?node('p',null,'下方仍显示当前窗口的已存记录；打开新知识后再点击生成。'):null,
  state.surface?.page?.page==='knowledge'?node('div',{class:'job-generation-status'},kinds.map(kind=>{const d=state.knowledge[kind];return node('span',null,[`${{wiki:'百科',mindmap:'导图',interview:'面试'}[kind]}：${phaseText[d?.job?.phase]??(d?.generated?'已保存':'尚未生成')}`,running(kind)?button('停止',()=>client.stopKnowledge(kind),state.busy):null,d?.job?.error?node('small',null,d.job.error):null]);})):null,
 ]);
 ui.settingsExtras=()=>node('div',{class:'job-model-permission'},[
  node('label',null,[node('input',{type:'checkbox',checked:modelDraft.enabled,disabled:locked.value,onChange:e=>modelDraft.enabled=e.target.checked}),'启用此模型连接（点击生成或开始查找时可能产生费用）']),
  node('p',null,state.model.hasKey?'密钥已保存；留空保留，输入新值后保存可替换。':'密钥仅由本机宿主保护保存。'),
  button('删除已存模型连接',async()=>{if(await client.clearModel()){modelDirty.value=false;syncDraft();}}),
  node('label',null,[node('input',{type:'checkbox',checked:draft.backgroundEnabled,disabled:locked.value,onChange:e=>draft.backgroundEnabled=e.target.checked}),'允许关闭或隐藏窗口后继续当前任务']),button('保存后台许可',save),
 ]);
 ui.connect=async()=>{await client.load();syncDraft();if(shell&&!host.preview){if(state.surface?.page?.page==='search'){await waitOverlay(true);await nextTick();await layout();}await client.surface('ready');if(state.surface?.page?.page==='analysis'){await query();queryRun=JSON.stringify([state.run,state.generation,state.revision,state.model.revision]);}}if(!disposed&&!host.preview)timer=setInterval(async()=>{if(disposed||polling)return;polling=true;try{await client.refresh();syncDraft();const key=JSON.stringify([state.run,state.generation,state.revision,state.model.revision]);if(state.surface?.page?.page==='analysis'&&queryRun!==key&&await query())queryRun=key;if(state.surface?.page?.page==='search')await perform(()=>waitOverlay(desiredOverlay()));}finally{polling=false;}},800);};
 ui.dispose=()=>{disposed=true;clearInterval(timer);clearTimeout(layoutTimer);resizeObserver?.disconnect();client.dispose();};
 return ui;
}
