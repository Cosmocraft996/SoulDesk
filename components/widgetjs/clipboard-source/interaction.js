// 只处理原历史列表快捷键；文字输入、弹框与输入法事件交回原控件。
export async function handleListKey(event,ui,scroll=()=>{}){
 if(event.defaultPrevented||event.isComposing||event.ctrlKey||event.altKey||event.metaKey||ui.view.value!=='list'||ui.locked.value)return false;
 const target=event.target,editing=target?.closest?.('textarea,[contenteditable="true"],[role="dialog"],.el-message-box,.el-popper');
 if(editing||target?.closest?.('input')&&event.key==='Delete'||target?.closest?.('button,a,[role="button"]'))return false;
 if(!['ArrowUp','ArrowDown','Enter','Delete'].includes(event.key)||!ui.state.items.length)return false;
 event.preventDefault();let index=ui.state.items.findIndex(row=>row.id===ui.state.selectedId);
 if(event.key==='ArrowUp'||event.key==='ArrowDown'){index=index<0?0:Math.max(0,Math.min(ui.state.items.length-1,index+(event.key==='ArrowDown'?1:-1)));const row=ui.state.items[index];if(await ui.run(()=>ui.client.select(row.id)))scroll(row);}
 else{const row=ui.state.items[index<0?0:index];if(event.key==='Enter')await ui.submit(row);else await ui.remove(row);}
 return true;
}
