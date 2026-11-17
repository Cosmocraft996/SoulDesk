import{createLibraryClient}from'./client.js';
import'./control.css';
// 此控件只操作元数据，切换时重建原视图；不自动拉取内容覆盖正在编辑的草稿。
export function installLibraryControl(host,options={}){
  if(host.preview)return()=>{};
  const wrapper=document.createElement('div');wrapper.className='souldesk-library';
  const toggle=document.createElement('button');toggle.className='souldesk-library-toggle';toggle.type='button';toggle.textContent='本地库';toggle.title='可选本地共享库，默认按实例独立保存';
  const dialog=document.createElement('dialog');dialog.className='souldesk-library-dialog';
  const title=document.createElement('h2');title.textContent='本地共享库';
  const explanation=document.createElement('p');explanation.textContent='默认各实例独立保存。连接共享库只替换当前内容来源，原私有数据保留；断开后恢复原数据。这里不提供云同步。';
  if(options.nativeKind==='todo')explanation.textContent+='只运行当前数据源的提醒，原私有后台暂停；隐藏视图仍继续当前后台。待办和大号日历可连接同一个库。';
  if(options.nativeKind==='pomodoro')explanation.textContent+='切换时原私有计时暂停，新库不自动开始；恢复私有或无人使用的库时保持暂停，由你继续。仍有其他实例使用的共享计时继续运行。场景、记录和计时设置共享。';
  const source=document.createElement('p'),message=document.createElement('p');message.setAttribute('role','status');
  const changed=document.createElement('p');changed.className='souldesk-library-changed';
  const buttons=document.createElement('div'),list=document.createElement('div');list.className='souldesk-library-list';
  const form=document.createElement('form'),name=document.createElement('input');name.maxLength=80;name.required=true;name.placeholder='新共享库名称';name.setAttribute('aria-label','新共享库名称');
  const create=document.createElement('button');create.type='submit';create.textContent='以当前已保存内容新建并连接';form.append(name,create);
  const reload=document.createElement('button');reload.type='button';reload.textContent='重新载入当前数据';
  const discard=document.createElement('button');discard.type='button';discard.textContent='放弃未保存草稿并重新载入';discard.hidden=true;
  const detach=document.createElement('button');detach.type='button';detach.textContent='断开并恢复本实例原数据';
  const refresh=document.createElement('button');refresh.type='button';refresh.textContent='刷新库列表';
  const close=document.createElement('button');close.type='button';close.textContent='关闭';
  buttons.append(reload,discard,detach,refresh,close);dialog.append(title,explanation,source,changed,form,list,message,buttons);wrapper.append(toggle,dialog);document.body.append(wrapper);
  let disposed=false,editing='',deleting='';
  const client=createLibraryClient(host,{beforeSwitch:async()=>{
    if([...document.querySelectorAll('dialog[open],[role="dialog"]')].some(other=>other!==dialog&&!dialog.contains(other)))throw Error('请先保存或关闭组件编辑窗口，再切换数据源');
    if(await host.beforeLibrarySwitch?.()===false)throw Error('当前草稿未保存，请重试保存或先导出');
  },reload(){location.reload();},changed:render});
  function button(text,action){const value=document.createElement('button');value.type='button';value.textContent=text;value.disabled=client.state.busy;value.onclick=()=>void perform(action);return value;}
  async function perform(action){try{await action();}catch(error){if(!disposed){message.textContent=String(error?.message||error);discard.hidden=false;}}}
  function render(){
    if(disposed)return;const state=client.state;source.textContent=state.status.binding?'当前共享库：'+state.status.binding.name+' · 版本 '+state.status.binding.revision:'当前：本实例独立保存';
    changed.textContent=state.changed?'共享库已有新版本。请先保存副本或处理草稿，再重新载入；旧版本写入会被拒绝。':'';toggle.textContent=state.changed?'本地库 · 已更新':state.status.binding?'共享库':'本地库';
    message.textContent=state.error;for(const element of[create,detach,refresh,reload,discard,close])element.disabled=state.busy;detach.hidden=!state.status.binding;
    list.replaceChildren();for(const library of state.libraries){const row=document.createElement('section'),label=document.createElement('strong');label.textContent=library.name+' · '+library.consumers+'实例 · 版本'+library.revision;row.append(label);
      row.append(button(state.status.binding?.id===library.id?'重新连接并载入':'连接并载入',()=>client.bind(library.id)));
      if(editing===library.id){const input=document.createElement('input');input.value=library.name;input.maxLength=80;input.setAttribute('aria-label','修改库名称');row.append(input,button('保存名称',async()=>{await client.rename(library,input.value);editing='';render();}),button('取消改名',()=>{editing='';render();}));}
      else row.append(button('改名',()=>{editing=library.id;deleting='';render();}));
      if(!library.consumers){if(deleting===library.id){const warning=document.createElement('p');warning.textContent='将永久删除这份未连接的共享内容，私有原数据保留。';row.append(warning,button('确认永久删除此库',async()=>{await client.remove(library);deleting='';render();}),button('取消删除',()=>{deleting='';render();}));}else row.append(button('删除未连接库',()=>{deleting=library.id;editing='';render();}));}
      list.append(row);
    }
  }
  toggle.onclick=()=>{dialog.showModal();void client.refresh();};close.onclick=()=>dialog.close();dialog.addEventListener('cancel',event=>{if(client.state.busy)event.preventDefault();});
  form.onsubmit=event=>{event.preventDefault();void perform(()=>client.create(name.value));};detach.onclick=()=>void perform(()=>client.bind(null));refresh.onclick=()=>void client.refresh();reload.onclick=()=>void perform(()=>client.reload());discard.onclick=()=>void perform(()=>client.reload(true));
  const timer=setInterval(()=>{if(!editing&&!deleting)void client.refresh();},5000);void client.refresh();
  const dispose=()=>{if(disposed)return;disposed=true;clearInterval(timer);client.dispose();wrapper.remove();};window.addEventListener('pagehide',dispose,{once:true});return dispose;
}
