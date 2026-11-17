// 仅查询共享库元数据；内容始终由原组件的storage接口和会话版本保护。
export function createLibraryClient(host,options={}){
  let disposed=false,refreshing=false;
  const state={status:{mode:'instance',binding:null,readRevision:null},libraries:[],changed:false,busy:false,error:''};
  const notice=()=>{if(!disposed)options.changed?.(state);};
  async function readMetadata(){
    const status=await host.request('library.status');if(disposed)return;
    const libraries=await host.request('library.list');if(disposed)return;
    if(!status||!['instance','shared'].includes(status.mode)||!Array.isArray(libraries))throw Error('共享库元数据无效');
    state.status=status;state.libraries=libraries;state.changed=!!status.binding&&(status.binding.contentRevision??status.binding.revision)!==status.readRevision;state.error='';notice();
  }
  async function beforeSwitch(){if(await options.beforeSwitch?.()===false)throw Error('当前草稿未保存，请先重试保存或导出');if(disposed)return false;return true;}
  async function mutate(work){
    if(disposed)throw Error('组件已关闭');if(host.preview)throw Error('预览不修改共享库');if(state.busy)throw Error('共享库操作尚未完成');
    state.busy=true;state.error='';notice();
    try{return await work();}catch(error){if(!disposed){state.error=String(error?.message||error);notice();}throw error;}
    finally{if(!disposed){state.busy=false;notice();}}
  }
  async function bind(id){const result=await host.request('library.bind',{id});if(!disposed){if(!result?.reloadRequired)throw Error('切换未确认，请重新载入确认数据源');options.reload?.();}return result;}
  return{state,
    async refresh(){if(disposed||host.preview||refreshing||state.busy)return;refreshing=true;try{await readMetadata();}catch(error){if(!disposed){state.error=String(error?.message||error);notice();}}finally{refreshing=false;}},
    bind(id){return mutate(async()=>{if(await beforeSwitch())return bind(id);});},
    create(name){return mutate(async()=>{if(!await beforeSwitch())return;const result=await host.request('library.create',{name});if(!disposed){if(result?.reloadRequired){options.reload?.();return result;}return bind(result.id);}});},
    rename(library,name){return mutate(async()=>{await host.request('library.rename',{id:library.id,expectedRevision:library.revision,name});if(!disposed)await readMetadata();});},
    remove(library){return mutate(async()=>{await host.request('library.remove',{id:library.id,expectedRevision:library.revision});if(!disposed)await readMetadata();});},
    reload(discard=false){return mutate(async()=>{if(discard||await beforeSwitch())if(!disposed)options.reload?.();});},
    dispose(){disposed=true;},
  };
}
