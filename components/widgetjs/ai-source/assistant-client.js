// 网页助手的界面状态；只调用固定宿主方法，不创建网页或读取登录资料。
const clone=value=>JSON.parse(JSON.stringify(value));
const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.entries(value).sort(([a],[b])=>a.localeCompare(b)).map(([k,v])=>[k,canonical(v)])):value;
const same=(a,b)=>JSON.stringify(canonical(a))===JSON.stringify(canonical(b));
export const platforms=['chatgpt','grok','gemini','deepseek','kimi','qwen','dou_bao','yuan_bao'];
export const defaultAssistantConfig=()=>({backgroundEnabled:false,shortcut:'Super+Alt+C',alwaysTop:false,pageCount:1,platforms:platforms.slice(0,4),position:'right',widthRatio:0.3,proxy:{mode:'system'},tipProgress:0});
const message=error=>error?.message??String(error);
const uncertain=error=>/HOST_TIMEOUT|超时|timeout|连接.*断|结果未确定/i.test(message(error));
export function createAssistantClient(host,{shell=false,changed=()=>{},sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms))}={}){
  let disposed=false,epoch=0,reading=false,pending;
  const state={revision:0,config:defaultAssistantConfig(),surface:null,nativeCleanupPending:false,nativeError:null,shortcutRegistered:false,shortcutError:null,loading:!host.preview,busy:false,uncertain:false,error:'',pollError:''};
  const emit=()=>{if(!disposed)changed(clone(state));};
  function apply(value){
    if(!value||!Number.isSafeInteger(value.revision)||value.revision<0||!value.config||!Array.isArray(value.config.platforms)||value.config.platforms.length!==4||value.config.platforms.some(p=>!platforms.includes(p)))throw Error('DATA_UNAVAILABLE: 网页助手状态无效，保留现有画面');
    if(value.revision<state.revision)return;
    const sameRevision=value.revision===state.revision;
    const oldSurface=state.surface;
    state.revision=value.revision;state.config=clone(value.config);
    state.surface=oldSurface&&value.surface&&sameRevision&&value.surface.serial<oldSurface.serial?oldSurface:clone(value.surface??null);
    for(const key of ['nativeCleanupPending','nativeError','shortcutRegistered','shortcutError'])state[key]=value[key]??(key.endsWith('Error')?null:false);
  }
  async function refresh(){
    if(disposed||host.preview||reading||state.busy)return false;
    const current=epoch;reading=true;
    try{const value=await host.request('assistant.status');if(disposed||current!==epoch)return false;apply(value);state.pollError='';return true;}
    catch(error){if(!disposed&&current===epoch)state.pollError=message(error);return false;}
    finally{reading=false;if(!disposed&&current===epoch){state.loading=false;emit();}}
  }
  async function sendConfig(){
    if(disposed||host.preview||state.busy||!pending)return false;
    const operation=pending,current=++epoch;state.busy=true;state.error='';emit();
    try{
      const value=await host.request('assistant.configure',clone(operation));
      if(disposed||current!==epoch)return false;
      apply(value);pending=undefined;state.uncertain=false;
      if(value.revision!==operation.expectedRevision+1||!same(value.config,operation.config)){state.error='原保存已完成，但当前设置随后有新改动；请重新核对';return false;}
      return true;
    }catch(error){
      if(!disposed&&current===epoch){state.error=message(error);state.uncertain=uncertain(error);if(!state.uncertain)pending=undefined;}
      return false;
    }finally{if(!disposed&&current===epoch){state.busy=false;emit();}}
  }
  async function save(config,expectedRevision=state.revision){
    if(disposed||host.preview||state.busy)return false;
    if(pending&&(pending.expectedRevision!==expectedRevision||!same(pending.config,config))){state.error='保存结果尚未确定，请先重试原保存请求；当前草稿保留';emit();return false;}
    pending??={requestId:crypto.randomUUID(),expectedRevision,config:clone(config)};
    return sendConfig();
  }
  async function action(type){
    if(!['open','hide','retry'].includes(type))throw Error('CAPABILITY_DENIED: 助手动作无效');
    if(disposed||host.preview||state.busy||state.uncertain&&type!=='hide')return false;
    const current=++epoch;state.busy=true;state.error='';emit();
    try{const value=await host.request('assistant.'+type,{expectedRevision:state.revision});if(disposed||current!==epoch)return false;apply(value);return true;}
    catch(error){if(!disposed&&current===epoch)state.error=message(error);return false;}
    finally{if(!disposed&&current===epoch){state.busy=false;emit();}}
  }
  async function overlay(id,visible){
    if(!shell||!['guide','settings','select0','select1','select2','select3'].includes(id))throw Error('CAPABILITY_DENIED: 当前视图不能操作该覆盖层');
    if(disposed||host.preview||state.busy||state.uncertain)return false;
    const current=++epoch;state.busy=true;state.error='';emit();
    try{
      const value=await host.request('assistant.overlay',{expectedRevision:state.revision,id,visible});
      if(disposed||current!==epoch)return false;apply(value);
      const presentation=value.surface?.presentation,serial=value.surface?.serial;
      if(!presentation||!Number.isSafeInteger(serial))throw Error('PRESENTATION_EXPIRED: 助手窗口已关闭');
      for(let i=0;i<100;i++){
        if(disposed||current!==epoch)return false;
        const s=state.surface;
        if(s?.presentation!==presentation)throw Error('PRESENTATION_EXPIRED: 助手窗口已被替换');
        if(s.phase==='failed'||state.nativeError)throw Error(s.error??state.nativeError??'助手显示失败');
        if(s.nativeSettled&&s.serial>=serial&&Array.isArray(s.overlays)&&s.overlays.includes(id)===visible)return true;
        await sleep(50);if(disposed||current!==epoch)return false;
        apply(await host.request('assistant.status'));
      }
      throw Error('HOST_TIMEOUT: 网页覆盖状态尚未确认，请重试；当前草稿保留');
    }catch(error){if(!disposed&&current===epoch)state.error=message(error);return false;}
    finally{if(!disposed&&current===epoch){state.busy=false;emit();}}
  }
  return{state,refresh,save,action,overlay,retrySave:sendConfig,
    quick:patch=>save({...clone(state.config),...clone(patch)}),
    async load(){if(host.preview){state.loading=false;emit();return true;}return refresh();},
    dispose(){disposed=true;epoch++;},
  };
}
