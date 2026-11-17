// 三款AI工具共享的界面状态；不包含模型密钥的普通持久化。
const copy=value=>JSON.parse(JSON.stringify(value));
const connection=()=>({revision:0,configured:false,enabled:false,apiBaseUrl:'',model:'',hasKey:false,job:null});
export function inputFor(variant,value={}){
  const input={content:typeof value.content==='string'?value.content.slice(0,20000):''};
  if(variant==='translate'){input.firstLang=value.firstLang??'zh';input.secondLang=value.secondLang??'en';}
  else if(variant==='mindmap')input.extra=typeof value.extra==='string'?value.extra.slice(0,1000):'';
  else input.chartType=value.chartType??'Flowchart';
  return input;
}
function documentFor(variant,value={}){
  return {version:1,...inputFor(variant,value),result:typeof value.result==='string'?value.result:'',theme:['light','dark','auto'].includes(value.theme)?value.theme:'auto',history:Array.isArray(value.history)?value.history.filter(item=>item&&typeof item.id==='string'&&typeof item.result==='string').slice(-20).map(item=>({id:item.id,input:inputFor(variant,item.input),result:item.result.slice(0,524288),state:['completed','failed','cancelled','interrupted'].includes(item.state)?item.state:'interrupted',error:typeof item.error==='string'?item.error.slice(0,1000):null,at:Number.isSafeInteger(item.at)?item.at:0})):[]};
}
export function createClient(host,variant,changed=()=>{}){
  let generation=0,disposed=false,pending,pendingConfig,pendingClear,saving=Promise.resolve(),jobInput=inputFor(variant);
  const state={connection:connection(),document:documentFor(variant),input:inputFor(variant),job:null,busy:false,uncertain:false,error:'',saveError:'',restoreNote:'',configUncertain:false,clearUncertain:false};
  const emit=()=>{if(!disposed)changed({...state});};
  const unknown=error=>/超时|timeout|连接.*断|结果未确定/i.test(error.message);
  function receive(job){state.job=copy(job);state.document.result=job.text??state.document.result;if(job.error)state.error=job.error;emit();}
  async function saveDocument(value){
    if(host.preview||disposed)return false;
    state.document=documentFor(variant,{...state.document,...value});emit();
    const snapshot=copy(state.document);while(new TextEncoder().encode(JSON.stringify(snapshot)).length>2*1024*1024&&snapshot.history.length)snapshot.history.shift();
    state.document.history=snapshot.history;
    const operation=saving.catch(()=>{}).then(async()=>{try{await host.request('storage.set',snapshot);if(!disposed){state.saveError='';emit();}return true;}catch(error){if(!disposed){state.saveError='内容未保存：'+error.message;emit();}return false;}});saving=operation;return operation;
  }
  async function remember(job=state.job,input=jobInput,preserveDraft=false){
    if(!job||job.state==='running')return;
    const history=state.document.history.filter(item=>item.id!==job.requestId);if(job.text)history.push({id:job.requestId,input:copy(input),result:job.text,state:job.state,error:job.error??null,at:Date.now()});
    await saveDocument({...(!preserveDraft?{result:job.text??state.document.result}:{}),history});
  }
  async function send(){
    const operation=pending;if(!operation||state.busy||disposed)return;const oldJob=state.job?copy(state.job):null,oldInput=copy(jobInput);
    const current=++generation;state.busy=true;state.error='';emit();
    try{const job=await host.request('ai.begin',operation);if(disposed||current!==generation)return;if(oldJob?.state==='running'&&oldJob.requestId!==job.requestId&&oldJob.text)await remember({...oldJob,state:'cancelled',error:'已被新生成请求替换'},oldInput,true);if(disposed||current!==generation)return;jobInput=inputFor(variant,job.input??operation.input);pending=undefined;state.uncertain=false;receive(job);await saveDocument({...state.input,result:job.text??''});if(job.state!=='running')await remember();}
    catch(error){if(!disposed&&current===generation){state.error=error.message;state.uncertain=unknown(error);if(!state.uncertain)pending=undefined;emit();}}
    finally{if(!disposed&&current===generation){state.busy=false;emit();}}
  }
  async function configure(draft){
    if(host.preview||disposed||state.busy)return false;
    if(pendingClear)throw Error('清除连接结果未确定，请先重试原清除请求');
    if(pendingConfig&&JSON.stringify(pendingConfig.draft)!==JSON.stringify(draft))throw Error('连接保存结果未确定，请先重试原设置');
    pendingConfig??={draft:copy(draft),args:{...copy(draft),requestId:crypto.randomUUID(),expectedRevision:state.connection.revision}};
    const current=++generation;state.busy=true;state.error='';emit();
    try{const value=await host.request('ai.configure',pendingConfig.args);if(disposed||current!==generation)return false;const wasRunning=state.job?.state==='running'||state.uncertain;state.connection=value;if(value.job){jobInput=inputFor(variant,value.job.input??jobInput);if(wasRunning){receive(value.job);await remember();}else state.job=value.job;}pendingConfig=undefined;pending=undefined;state.uncertain=false;state.configUncertain=false;emit();return true;}
    catch(error){if(!disposed){state.error=error.message;state.configUncertain=unknown(error);if(!state.configUncertain)pendingConfig=undefined;emit();}return false;}
    finally{if(!disposed&&current===generation){state.busy=false;emit();}}
  }
  return {state,saveDocument,configure,async load(){
    if(host.preview||disposed)return;const current=generation;
    const values=await Promise.allSettled([host.request('ai.status'),host.request('storage.get')]);if(disposed||current!==generation)return;
    if(values[0].status==='fulfilled'){state.connection=values[0].value;state.job=values[0].value.job??null;}else state.error=values[0].reason.message;
    if(values[1].status==='fulfilled')state.document=documentFor(variant,values[1].value);else state.saveError=values[1].reason.message;
    const previous=state.document.history.find(item=>item.id===state.job?.requestId);jobInput=inputFor(variant,state.job?.input??previous?.input??state.document);
    if(state.job?.state==='running'){state.input=copy(jobInput);state.document.result=state.job.text??'';}else{state.input=inputFor(variant,state.document);if(state.job?.text){state.restoreNote='保留已保存草稿；上次模型结果可从保存的结果中读取';if(!previous||previous.result!==state.job.text||previous.state!==state.job.state)await remember(state.job,jobInput,true);}else if(state.job?.state==='interrupted'){state.restoreNote='上次生成已中断；已保存文本可能为部分结果，未重复发送。';}else if(state.job?.state==='failed'&&state.job.error){state.error=state.job.error;}}
    emit();
  },async generate(input){
    if(host.preview||disposed)return;if(state.uncertain)throw Error('上次生成结果未确定，请先重试原请求');if(state.busy)return;
    if(state.configUncertain)throw Error('连接保存结果未确定，请先重试保存');
    state.input=inputFor(variant,input);pending={requestId:crypto.randomUUID(),configRevision:state.connection.revision,input:copy(state.input)};await send();
  },retry:send,async poll(){
    if(host.preview||disposed||state.busy||state.uncertain||!state.job||state.job.state!=='running')return;const id=state.job.requestId,current=generation;
    try{const job=await host.request('ai.poll',{requestId:id});if(disposed||current!==generation||state.job?.requestId!==id)return;state.error='';receive(job);if(job.state!=='running')await remember();}
    catch(error){if(!disposed&&current===generation){state.error=error.message;emit();}}
  },async cancel(){
    if(host.preview||disposed)return;const id=pending?.requestId??state.job?.requestId;if(!id)return;const current=++generation,attemptInput=pending?.input;
    try{const job=await host.request('ai.cancel',{requestId:id});if(disposed||current!==generation)return;if(job.input||attemptInput)jobInput=inputFor(variant,job.input??attemptInput);pending=undefined;state.uncertain=false;state.busy=false;receive(job);await remember();}
    catch(error){if(!disposed){state.error=error.message;emit();}}
  },async clear(){
    if(host.preview||disposed||state.busy||state.configUncertain&&!pendingClear)return false;pendingClear??={requestId:crypto.randomUUID(),expectedRevision:state.connection.revision};const current=++generation;state.busy=true;state.error='';emit();
    try{const value=await host.request('ai.clear',pendingClear);if(disposed||current!==generation)return false;const wasRunning=state.job?.state==='running'||state.uncertain;state.connection=value;if(value.job){jobInput=inputFor(variant,value.job.input??jobInput);if(wasRunning){receive(value.job);await remember();}else state.job=value.job;}pendingClear=undefined;pending=undefined;state.uncertain=false;state.configUncertain=false;state.clearUncertain=false;emit();return true;}
    catch(error){if(!disposed){state.error=error.message;state.configUncertain=state.clearUncertain=unknown(error);if(!state.configUncertain)pendingClear=undefined;emit();}return false;}
    finally{if(!disposed&&current===generation){state.busy=false;emit();}}
  },async dispose(){if(disposed)return;const id=pending?.requestId??(state.job?.state==='running'?state.job.requestId:undefined);disposed=true;++generation;if(id&&!host.preview){try{await host.request('ai.cancel',{requestId:id});}catch{}}await saving;},flush:()=>saving};
}
