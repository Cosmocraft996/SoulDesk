// 剪切板固定宿主客户端：界面状态不承担采集、提醒或系统输入。
const copy=value=>value===undefined?undefined:JSON.parse(JSON.stringify(value));
const unknown=error=>/超时|timeout|结果未确定|连接.*断/i.test(error?.message??String(error));
export function initialState(kind='history'){
 return{kind,status:{revision:0,configRevision:0,captureEnabled:false,shortcutEnabled:false,pasteEnabled:false,shortcut:kind==='search'?'Super+Alt+S':'Super+Alt+V',historyExpiredHours:12,searchPlatform:'google',count:0,actionRevision:0,actionError:null},surface:null,items:[],selectedId:'',draft:null,dirty:false,stale:false,keyword:'',favoritesOnly:false,page:1,pageSize:10,total:0,hasMore:false,loading:false,loadingMore:false,busy:false,mutationUncertain:false,actionUncertain:false,lastAction:null,error:'',notice:'',searchContent:'',searchDirty:false};
}
export function createClient(host,kind='history',changed=()=>{},state=initialState(kind),{popup=true}={}){
 let disposed=false,queryGeneration=0,selectionGeneration=0,pendingMutation,pendingAction,seenSerial=0,polling=false;
 const emit=()=>{if(!disposed)changed(state);};
 const available=()=>!host.preview&&!disposed;
 function fail(error){if(!disposed){state.error=error?.message??String(error);emit();}}
 function guard(){if(pendingMutation||pendingAction||state.actionUncertain)throw Error('上次操作结果未确定，请先读取结果或重试原请求');if(state.dirty)throw Error('请先保存或放弃正在编辑的草稿');}
 function acceptStatus(value){const before=state.status.actionRevision;state.status={...state.status,...copy(value)};if(Number.isSafeInteger(before)&&Number.isSafeInteger(value.actionRevision))state.status.actionRevision=Math.max(before,value.actionRevision);const last=value.lastAction;if(last&&(!pendingAction||pendingAction.requestId===last.requestId)&&JSON.stringify(last)!==JSON.stringify(state.lastAction))consumeAction(last);}
 function applyDraft(row){state.selectedId=row?.id??'';state.draft=row?copy(row):null;state.dirty=false;state.stale=false;}
 async function refresh(clearError=false){
  if(!available()||!popup||kind!=='history')return;const generation=++queryGeneration;state.loading=true;if(clearError===true)state.error='';emit();
  try{const value=await host.request('clipboard.query',{keyword:state.keyword,page:1,size:state.pageSize,favoritesOnly:state.favoritesOnly});if(disposed||generation!==queryGeneration)return;
   state.items=copy(value.items);state.page=1;state.total=value.total;state.hasMore=value.hasMore;state.status.revision=value.revision;
   if(state.draft){const current=state.items.find(r=>r.id===state.selectedId);if(state.dirty){if(!current||current.revision!==state.draft.revision)state.stale=true;}else if(current)applyDraft(current);}
  }catch(error){if(!disposed&&generation===queryGeneration)fail(error);}finally{if(!disposed&&generation===queryGeneration){state.loading=false;emit();}}
 }
 async function select(id){
  if(!available())return false;if(state.dirty&&state.selectedId!==id)throw Error('请先保存或放弃草稿');if(state.dirty)return true;
  const generation=++selectionGeneration;
  try{const row=state.items.find(r=>r.id===id)??await host.request('clipboard.get',{id});if(disposed||generation!==selectionGeneration)return false;applyDraft(row);emit();return true;}catch(error){fail(error);return false;}
 }
 async function sendMutation(){
  if(!available()||!pendingMutation||state.busy)return false;const pending=pendingMutation;state.busy=true;state.error='';emit();
  try{const value=await host.request('clipboard.mutate',pending.args);if(disposed)return false;acceptStatus(value);pendingMutation=undefined;state.mutationUncertain=false;
   if(pending.kind==='edit-draft'){
    state.dirty=false;
    try{applyDraft(await host.request('clipboard.get',{id:pending.args.command.id}));}catch(error){state.stale=true;state.error='保存已提交，但重读失败，请刷新：'+error.message;}
   }
   if(pending.kind==='configure')state.settingsSavedRevision=value.configRevision;
   state.notice='已保存';await refresh();emit();return true;
  }catch(error){if(!disposed){state.mutationUncertain=unknown(error);if(!state.mutationUncertain)pendingMutation=undefined;fail(error);}return false;}
  finally{if(!disposed){state.busy=false;emit();}}
 }
 async function mutate(command,operation='ordinary'){
  if(!available())return false;if(pendingMutation||pendingAction)throw Error('上次操作结果未确定，请先重试原请求');if(state.busy)return false;
  pendingMutation={args:{requestId:crypto.randomUUID(),command:copy(command)},kind:operation};return sendMutation();
 }
 function consumeAction(result){
  state.lastAction=copy(result);if(Number.isSafeInteger(result.actionRevision))state.status.actionRevision=result.actionRevision;
  if(result.state==='pending'){state.actionUncertain=true;state.canRetryAction=!!pendingAction;state.notice='上次动作仍在执行，请稍后读取结果';return;}
  pendingAction=undefined;state.actionUncertain=false;state.canRetryAction=false;state.error=result.error??'';
  state.notice=result.copy==='copied'?(result.paste==='submitted'?'已复制，粘贴按键已提交':'已复制到剪贴板'):'动作未确认完成';
  if(result.state==='unknown')state.error=result.error??'动作结果不确定，未重复执行，请核对实际结果';
 }
 async function sendAction(){
  if(!available()||!pendingAction||state.busy)return false;const args=copy(pendingAction);state.busy=true;state.error='';emit();
  try{const value=await host.request('clipboard.act',args);if(disposed)return false;consumeAction(value);await refresh();emit();return value.state==='completed';}
  catch(error){if(!disposed){state.actionUncertain=unknown(error);if(!state.actionUncertain)pendingAction=undefined;state.canRetryAction=!!pendingAction;fail(error);}return false;}
  finally{if(!disposed){state.busy=false;emit();}}
 }
 return{state,refresh,select,
  async load(){
   if(!available())return;state.loading=true;emit();const operations=[host.request('clipboard.status')];if(popup)operations.push(host.request('clipboard.surface'));
   const values=await Promise.allSettled(operations);if(disposed)return;const errors=[];
   if(values[0].status==='fulfilled')acceptStatus(values[0].value);else errors.push(values[0].reason.message);
   if(popup){if(values[1].status==='fulfilled')state.surface=copy(values[1].value);else{state.surface=null;errors.push(values[1].reason.message);}}
   if(popup&&kind==='history')await refresh();state.loading=false;state.error=[...errors,state.error].filter(Boolean).join('；');emit();
  },
  async poll(){
   if(!available()||polling||state.busy)return;polling=true;
   try{const before=state.status.revision;const value=await host.request('clipboard.status');if(disposed)return;acceptStatus(value);
    if(popup&&kind==='history'&&value.revision!==before){if(state.page===1)await refresh();else state.notice='历史有更新，可点击刷新查看';}
    if(popup&&kind==='search'){const sample=await host.request('clipboard.peek');if(disposed)return;if(sample.serial!==seenSerial){seenSerial=sample.serial;if(!state.searchDirty)state.searchContent=sample.text??'';}}
    emit();
   }catch(error){fail(error);}finally{polling=false;}
  },
  async loadMore(){
   if(!available()||state.loadingMore||state.loading||!state.hasMore)return;const generation=queryGeneration,page=state.page+1;state.loadingMore=true;emit();
   try{const value=await host.request('clipboard.query',{keyword:state.keyword,page,size:state.pageSize,favoritesOnly:state.favoritesOnly});if(disposed||generation!==queryGeneration)return;
    const ids=new Set(state.items.map(r=>r.id));state.items.push(...copy(value.items).filter(row=>!ids.has(row.id)));state.page=page;state.total=value.total;state.hasMore=value.hasMore;
   }catch(error){if(generation===queryGeneration)fail(error);}finally{if(!disposed){state.loadingMore=false;emit();}}
  },
  async setQuery(keyword,favoritesOnly=state.favoritesOnly){state.keyword=String(keyword).slice(0,200);state.favoritesOnly=!!favoritesOnly;return refresh();},
  edit(patch){if(disposed||state.busy||state.actionUncertain||state.mutationUncertain)return;if('searchContent'in patch){state.searchContent=String(patch.searchContent).slice(0,50000);state.searchDirty=true;}else if(state.draft){for(const name of['content','labels','favorite'])if(name in patch)state.draft[name]=copy(patch[name]);state.dirty=true;}emit();},
  discard(){const row=state.items.find(r=>r.id===state.selectedId);applyDraft(row??state.draft);state.searchDirty=false;emit();},
  async saveDraft(){if(!state.draft||!state.dirty)return true;return mutate({type:'edit',id:state.draft.id,expectedItemRevision:state.draft.revision,content:state.draft.content,labels:copy(state.draft.labels),favorite:state.draft.favorite},'edit-draft');},
  async saveSettings(draft){guard();return mutate({type:'configure',expectedRevision:draft.configRevision,captureEnabled:!!draft.captureEnabled,shortcutEnabled:!!draft.shortcutEnabled,pasteEnabled:!!draft.pasteEnabled,shortcut:String(draft.shortcut??''),historyExpiredHours:Number(draft.historyExpiredHours),searchPlatform:['google','bing','baidu'].includes(draft.searchPlatform)?draft.searchPlatform:'google'},'configure');},
  async favorite(row,value){guard();return mutate({type:'edit',id:row.id,expectedItemRevision:row.revision,content:row.content,labels:copy(row.labels),favorite:!!value});},
  async remove(row){guard();return mutate({type:'delete',id:row.id,expectedItemRevision:row.revision});},
  async clear(){guard();return mutate({type:'clear',expectedRevision:state.status.revision});},
  retryMutation:sendMutation,
  async act(row,paste=false){
   if(!available())return false;guard();if(state.busy)return false;if(!state.surface?.presentationId||!Number.isSafeInteger(state.status.actionRevision))throw Error('窗口或动作回执不可用，请重新打开');
   pendingAction={requestId:crypto.randomUUID(),actionRevision:state.status.actionRevision,configRevision:state.status.configRevision,presentationId:state.surface.presentationId,itemId:row.id,itemRevision:row.revision,paste:!!paste};return sendAction();
  },retryAction:sendAction,
  async readAction(){const id=pendingAction?.requestId??state.lastAction?.requestId;if(!available()||!id)return;try{const value=await host.request('clipboard.actionStatus',{requestId:id});if(!disposed){consumeAction(value);await refresh();emit();}}catch(error){fail(error);}},
  async open(){if(available())return host.request('clipboard.open');},async hide(){guard();if(available())return host.request('clipboard.hide');},
  async hover(entered){if(available())try{await host.request('clipboard.hover',{entered:!!entered});}catch(error){if(!disposed)fail(error);}},
  async search(){if(available())return host.request('clipboard.search',{content:state.searchContent});},
  async tool(toolKind){if(!['mindmap','translate'].includes(toolKind))throw Error('不支持的工具');if(available())return host.request('clipboard.tool',{kind:toolKind,content:state.searchContent});},
  dispose(){disposed=true;++queryGeneration;++selectionGeneration;}
 };
}
