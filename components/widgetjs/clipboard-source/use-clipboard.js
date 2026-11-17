import{reactive,ref,computed,onMounted,onBeforeUnmount}from'vue';import{ElMessageBox}from'element-plus';import{createClient,initialState}from'./client.js';
export function useClipboard(host,kind,popup){
 const state=reactive(initialState(kind)),view=ref('list'),loaded=ref(host.preview),client=createClient(host,kind,()=>{},state,{popup}),locked=computed(()=>state.busy||state.actionUncertain||state.mutationUncertain);let timer,disposed=false;
 async function run(action){try{return await action();}catch(error){state.error=error.message??String(error);return false;}}
 function navigate(next){if(state.dirty||locked.value){state.error=state.dirty?'请先保存或放弃草稿':'上次操作结果未确定';return false;}view.value=next;return true;}
 async function openDetail(row){if(await run(()=>client.select(row.id)))navigate('detail');}
 async function remove(row){if(locked.value)return false;try{await ElMessageBox.confirm('确定删除该记录？','提示',{confirmButtonText:'确定',cancelButtonText:'取消',type:'warning',appendTo:'#clipboard'});return await run(()=>client.remove(row));}catch(error){if(error!=='cancel'&&error!=='close')state.error=error.message??String(error);return false;}}
 async function submit(row,paste=state.status.pasteEnabled&&state.surface?.pasteTargetAvailable){return run(()=>client.act(row,!!paste));}
 async function search(){const result=await run(()=>client.search());if(result?.submitted)state.notice='搜索请求已交给系统浏览器';return result;}
 async function settings(){if(!host.preview)return run(()=>host.request('clipboard.settings'));}
 function discardDetail(){if(locked.value){state.error='请先确定上次操作结果';return;}client.discard();navigate('list');}
 function keyboard(event){if(event.key!=='Escape'||event.isComposing||event.defaultPrevented||event.target?.closest?.('[role="dialog"],.el-message-box,.el-popper'))return;event.preventDefault();if(view.value==='list')void run(()=>client.hide());else navigate('list');}
 onMounted(async()=>{if(popup)document.addEventListener('keydown',keyboard);await client.load();if(disposed)return;loaded.value=true;await client.poll();timer=setInterval(()=>{void client.poll();},500);});onBeforeUnmount(()=>{disposed=true;clearInterval(timer);document.removeEventListener('keydown',keyboard);client.dispose();});
 return{host,kind,popup,state,view,loaded,client,locked,run,navigate,openDetail,remove,submit,search,settings,discardDetail};
}
