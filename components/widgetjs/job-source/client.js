// AI直聘界面仅发固定宿主请求；读取和恢复状态不会开始采集或生成。
const copy=value=>JSON.parse(JSON.stringify(value));
const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.entries(value).sort(([a],[b])=>a.localeCompare(b)).map(([key,item])=>[key,canonical(item)])):value;
const equal=(a,b)=>JSON.stringify(canonical(a))===JSON.stringify(canonical(b));
const errorText=error=>error?.message??String(error);
const uncertain=error=>/HOST_TIMEOUT|timeout|超时|连接.*断|结果未确定/i.test(errorText(error));
export const kinds=['wiki','mindmap','interview'];
export const defaultConfig=()=>({search:{skills:'',selfIntroduction:'',desiredSalary:'',location:'',searchKeyword:'',disadvantages:'',advantages:['双休','不加班','早九晚五']},backgroundEnabled:false});
export function createJobClient(host,{shell=false,changed=()=>{},uuid=()=>crypto.randomUUID()}={}){
  const state={revision:0,generation:0,config:defaultConfig(),model:{revision:0,configured:false,enabled:false,hasKey:false,apiBaseUrl:'',model:''},run:null,execution:null,presentation:null,browser:null,surface:null,query:null,knowledge:{},loading:!host.preview,busy:false,pending:false,lastCompleted:'',error:'',pollError:''};
  let disposed=false,epoch=0,reading=false,operation,queryEpoch=0;
  const emit=()=>{if(!disposed)changed(copy(state));};
  function apply(value){
    if(!value||!Number.isSafeInteger(value.revision)||!Number.isSafeInteger(value.generation)||!value.config?.search)throw Error('DATA_UNAVAILABLE: 求职状态无效，已保留现有画面');
    if(value.revision<state.revision||value.generation<state.generation)return;
    for(const key of ['revision','generation','config','run','execution','presentation','browser','model'])if(key in value)state[key]=copy(value[key]);
    if('surface'in value)state.surface=copy(value.surface);
  }
  function knowledge(value){
    if(!value||!kinds.includes(value.kind)||!Number.isSafeInteger(value.revision)||!Array.isArray(value.messages))throw Error('DATA_UNAVAILABLE: 知识记录无效');
    const old=state.knowledge[value.kind];if(old&&old.contextId!==value.contextId)throw Error('PRESENTATION_EXPIRED: 知识上下文已改变');
    if(!old||old.revision<=value.revision)state.knowledge[value.kind]=copy(value);
  }
  async function refresh(){
    if(disposed||host.preview||reading||state.busy)return false;
    const current=epoch;reading=true;
    try{
      const v=await host.request('jobs.status');if(disposed||epoch!==current)return false;apply(v);
      if(shell){const info=await host.request('jobs.surface.info');if(disposed||epoch!==current)return false;state.surface=copy(info.surface);
        if(info.surface?.page?.page==='knowledge'){const values=await Promise.all(kinds.map(kind=>host.request('jobs.knowledge.get',{kind})));if(disposed||epoch!==current)return false;values.forEach(knowledge);}}
      state.pollError='';return true;
    }catch(error){if(!disposed&&epoch===current)state.pollError=errorText(error);return false;}
    finally{reading=false;if(!disposed&&epoch===current){state.loading=false;emit();}}
  }
  async function send(op,retry=false,cancel=false){
    if(disposed||host.preview||state.busy)return false;
    if(operation&&!retry&&!cancel){state.error='上次操作结果尚未确定，请先重试原请求；当前草稿保留';emit();return false;}
    const previous=operation;
    const current=++epoch;state.busy=true;state.error='';emit();
    try{
      const v=await host.request(op.method,copy(op.args));if(disposed||epoch!==current)return false;
      if(op.method.startsWith('jobs.knowledge.'))knowledge(v);else apply(v);
      const sameCancelledRequest=previous?.args.requestId===op.args.requestId&&(op.method==='jobs.stop'&&previous.method==='jobs.start'||op.method==='jobs.knowledge.stop'&&previous.method==='jobs.knowledge.generate'&&previous.args.kind===op.args.kind);
      operation=cancel&&previous&&!sameCancelledRequest?previous:undefined;state.pending=!!operation;state.lastCompleted=op.method;
      if(op.method==='jobs.configure'&&(v.revision!==op.args.expectedRevision+1||!equal(v.config,op.args.config))){state.error='原保存已完成，当前求职设置随后有新改动；请核对后重新读取';return false;}
      if(op.method==='jobs.model.configure'&&(v.model.revision!==op.args.expectedRevision+1||v.model.apiBaseUrl!==op.args.apiBaseUrl||v.model.model!==op.args.model||v.model.enabled!==op.args.enabled)){state.error='原保存已完成，当前模型设置随后有新改动；请核对后重新读取';return false;}
      return true;
    }catch(error){if(!disposed&&epoch===current){state.error=errorText(error);state.pending=cancel?!!previous:op.retryable&&uncertain(error);operation=cancel?previous:state.pending?op:undefined;}return false;}
    finally{if(!disposed&&epoch===current){state.busy=false;emit();}}
  }
  function mutate(method,args,retryable=true){return send({method,args:copy(args),retryable});}
  const client={state,refresh,load:async()=>{if(host.preview){state.loading=false;emit();return true;}return refresh();},
    configure:(config,expectedRevision=state.revision)=>mutate('jobs.configure',{requestId:uuid(),expectedRevision,config}),
    model:(form,expectedRevision=state.model.revision)=>mutate('jobs.model.configure',{requestId:uuid(),expectedRevision,apiBaseUrl:form.url.replace(/\/chat\/completions\/?$/,''),model:form.model,enabled:form.enabled,...(form.apiKey?{apiKey:form.apiKey}:state.model.hasKey?{}:{apiKey:''})}),
    clearModel:()=>mutate('jobs.model.clear',{requestId:uuid(),expectedRevision:state.model.revision}),
    start:jobId=>mutate(jobId?'jobs.rerate':'jobs.start',{requestId:uuid(),configRevision:state.revision,generation:state.generation,modelRevision:state.model.revision,...(jobId?{jobId}:{})}),
    stop:()=>state.run?send({method:'jobs.stop',args:{requestId:state.run.id},retryable:false},false,true):Promise.resolve(false),
    clear:()=>mutate('jobs.clear',{expectedGeneration:state.generation}),
    open:page=>mutate('jobs.open',page,false),
    generate(kind,text){if(!kinds.includes(kind))throw Error('知识类型无效');const d=state.knowledge[kind];if(!d)return Promise.resolve(false);return mutate('jobs.knowledge.generate',{kind,requestId:uuid(),expectedRevision:d.revision,modelRevision:state.model.revision,...(kind==='interview'?{text}:{})});},
    clearKnowledge:kind=>{const d=state.knowledge[kind];return d?mutate('jobs.knowledge.clear',{kind,requestId:uuid(),expectedRevision:d.revision}):Promise.resolve(false);},
    stopKnowledge:kind=>{const d=state.knowledge[kind];return d?.job?send({method:'jobs.knowledge.stop',args:{kind,requestId:d.job.id},retryable:false},false,true):Promise.resolve(false);},
    retry:()=>operation?send(operation,true):Promise.resolve(false),
    async query(filter){if(disposed||host.preview)return false;const current=epoch,seq=++queryEpoch;try{const v=await host.request('jobs.query',filter);if(disposed||epoch!==current||seq!==queryEpoch)return false;apply(v);state.query=copy(v);state.pollError='';emit();return true;}catch(error){if(!disposed&&current===epoch&&seq===queryEpoch){state.pollError=errorText(error);emit();}return false;}},
    async surface(method,args=null){if(!shell||disposed||host.preview)return null;const v=await host.request('jobs.surface.'+method,args);if(disposed)return null;if(v.surface)state.surface=copy(v.surface);emit();return copy(v.surface??null);},
    fail(error){state.error=errorText(error);emit();},
    dispose(){disposed=true;epoch++;},
  };
  return client;
}
