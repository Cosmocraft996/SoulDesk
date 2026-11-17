import{computed,shallowRef,ref,onMounted,onBeforeUnmount}from'vue';
import{createAssistantClient}from'./assistant-client.js';import{AiUtils,labels}from'./assistant-services.js';
export function useAssistant(host,shell){
  const state=shallowRef(),settingsOpen=ref(false),notice=ref('');let timer,polling=false,disposed=false;
  const client=createAssistantClient(host,{shell,changed:value=>state.value=value});state.value={...client.state};
  const locked=computed(()=>host.preview||state.value.loading||state.value.busy||state.value.uncertain);
  const error=computed(()=>state.value.error||state.value.nativeError||state.value.pollError||state.value.surface?.error||'');
  async function settings(open=true){
    if(host.preview||state.value.busy||state.value.uncertain)return false;
    if(shell&&(open||state.value.surface?.phase!=='failed'&&!state.value.nativeError)&&!await client.overlay('settings',open))return false;
    settingsOpen.value=open;return true;
  }
  async function platform(index,value){
    if(locked.value||!AiUtils.aiServices.some(p=>p.value===value))return false;
    const values=[...state.value.config.platforms];values[index]=value;return client.quick({platforms:values});
  }
  async function guide(){if(await client.quick({tipProgress:Math.min(3,state.value.config.tipProgress+1)}))notice.value='';}
  onMounted(async()=>{await client.load();if(disposed||host.preview)return;timer=setInterval(async()=>{if(disposed||polling)return;polling=true;try{await client.refresh();}finally{polling=false;}},500);});
  onBeforeUnmount(()=>{disposed=true;clearInterval(timer);client.dispose();});
  return{host,shell,state,client,locked,error,settingsOpen,settings,platform,guide,notice,AiUtils,t:key=>labels[key]??key,
    phase:computed(()=>{const s=state.value.surface;if(s?.phase==='ready'&&!s.nativeSettled)return'正在显示';if(s?.phase==='hidden'&&!s.nativeSettled)return'正在收起';return{opening:'正在打开',ready:'已打开',hidden:'后台保留',failed:'显示失败'}[s?.phase]??(state.value.nativeCleanupPending?'正在关闭':'已关闭');})};
}
