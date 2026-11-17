import { connectHost } from '../../../sdk/host.js';
import { zipSync } from 'fflate';
import { convertBatch, validateOptions } from './conversion.js';
import { convertImage } from './image.js';
import { verifyConversion } from './verify.js';
import './style.css';
const $=id=>document.getElementById(id),host=await connectHost();
let settings=validateOptions(await host.request('settings.get')),files=[],results=[],output=null,busy=false,disposed=false;
const states={pending:'待转换',converting:'转换中',success:'已转换',error:'失败'};
function notify(message){$('status').textContent=message;}
function fill(){ $('format').value=settings.format;$('width').value=settings.width||'';$('height').value=settings.height||'';$('aspect').checked=settings.keepAspectRatio; }
function options(){return validateOptions({format:$('format').value,width:Number($('width').value),height:Number($('height').value),keepAspectRatio:$('aspect').checked});}
function toggle(value){busy=value;for(const element of document.querySelectorAll('button,input,select'))element.disabled=value||host.preview;$('convert').disabled=value||host.preview||!files.length;}
function render(){
  $('files').replaceChildren();
  files.forEach((item,index)=>{const row=document.createElement('tr');
    const name=document.createElement('td');name.textContent=item.file.name;
    const size=document.createElement('td');size.textContent=(item.file.size/1024).toFixed(1)+' KB';
    const status=document.createElement('td');status.textContent=states[item.state]+(item.error?'：'+item.error:'');status.dataset.state=item.state;
    const action=document.createElement('td'),remove=document.createElement('button');remove.textContent='移除';remove.type='button';remove.disabled=busy||host.preview;remove.onclick=()=>{files.splice(index,1);output=null;$('retry-save').hidden=true;render();toggle(false);};action.append(remove);row.append(name,size,status,action);$('files').append(row);
  });
}
function add(incoming){
  if(busy||host.preview)return;
  const next=[...files];
  for(const file of incoming){
    if(!file.type.startsWith('image/')&&!/\.(heic|heif|jpg|jpeg|png|webp|bmp|ico|gif|avif|svg)$/i.test(file.name))throw new Error('仅接收图片文件：'+file.name);
    if(!next.some(item=>item.file.name===file.name&&item.file.size===file.size))next.push({file,state:'pending',error:''});
  }
  if(next.length>32||next.reduce((sum,item)=>sum+item.file.size,0)>16*1024*1024)throw new Error('最多32张图片、合计16 MiB，请分批转换');
  files=next;results=[];output=null;$('retry-save').hidden=true;render();toggle(false);notify('已加入'+files.length+'张图片，文件仅在当前视图保留');
}
function decodeFile(file){const decoded=atob(file.data),data=Uint8Array.from(decoded,value=>value.charCodeAt(0));return new File([data],file.name,{type:file.mime});}
function base64(data){let value='';for(let offset=0;offset<data.length;offset+=16384)value+=String.fromCharCode(...data.subarray(offset,offset+16384));return btoa(value);}
async function saveOutput(){
  if(!output||host.preview)return;
  toggle(true);notify('请选择保存位置…');
  try { const saved=await host.request('documents.save',{fileName:output.name,extensions:[output.extension],data:base64(output.data)});notify(saved?'已保存'+results.filter(item=>item.blob).length+'张转换图片'+(results.some(item=>item.error)?'；其余失败项见列表':''):'已取消保存；转换结果保留，可再次保存'); }
  catch(error){notify('未保存：'+String(error?.message||error)+'；转换结果保留');}
  finally{toggle(false);}
}
$('pick').onclick=async()=>{
  if(busy||host.preview)return;toggle(true);
  try{const selected=await host.request('documents.open',{fileName:'图片.png',extensions:['heic','heif','jpg','jpeg','png','webp','bmp','ico','gif','avif','svg'],multiple:true});toggle(false);if(selected)add(selected.map(decodeFile));else notify('已取消选择，原文件列表保留');}
  catch(error){notify(String(error?.message||error));}finally{toggle(false);}
};
document.addEventListener('dragover',event=>{event.preventDefault();if(!busy&&!host.preview)$('pick').classList.add('dragging');});
document.addEventListener('dragleave',()=>$('pick').classList.remove('dragging'));
document.addEventListener('drop',event=>{event.preventDefault();$('pick').classList.remove('dragging');try{add([...event.dataTransfer.files]);}catch(error){notify(String(error?.message||error));}});
$('clear').onclick=()=>{if(busy)return;files=[];results=[];output=null;$('retry-save').hidden=true;render();toggle(false);notify('列表已清空，原文件保留');};
$('retry-save').onclick=()=>void saveOutput();
async function saveSettings(){settings=await host.request('settings.update',options());fill();}
$('save-settings').onclick=async()=>{if(busy||host.preview)return;toggle(true);try{await saveSettings();notify('转换设置已保存');}catch(error){notify('设置未保存：'+String(error?.message||error));}finally{toggle(false);}};
$('options').onsubmit=async event=>{
  event.preventDefault();if(busy||host.preview||!files.length)return;toggle(true);output=null;$('retry-save').hidden=true;
  try{
    await saveSettings();notify('正在转换；HEIC需本地解码，请稍候…');
    results=await convertBatch(files.map(item=>item.file),settings,convertImage,(index,state)=>{if(disposed)return;files[index].state=state;files[index].error='';render();});
    if(disposed)return;
    results.forEach((item,index)=>{files[index].error=item.error;});render();
    const successes=results.filter(item=>item.blob);if(!successes.length){notify('全部转换失败，详情见列表；可修改设置后重试');return;}
    if(files.length===1){const item=successes[0];output={name:item.name,extension:settings.format,data:new Uint8Array(await item.blob.arrayBuffer())};}
    else{const entries={};for(const item of successes)entries[item.name]=new Uint8Array(await item.blob.arrayBuffer());output={name:'converted_images.zip',extension:'zip',data:zipSync(entries)};}
    if(output.data.length>32*1024*1024){output=null;throw new Error('输出文件超过32 MiB，请缩小尺寸或分批转换');}
    $('retry-save').hidden=false;await saveOutput();
  }catch(error){notify('转换或设置失败：'+String(error?.message||error));}finally{if(!disposed)toggle(false);}
};
fill();toggle(false);notify(host.preview?'预览 · 不读取文件、不转换、不保存':'选择图片后转换，输出副本保存到用户选择的位置');
await host.ready();window.addEventListener('pagehide',()=>{disposed=true;files=[];results=[];output=null;},{once:true});
// 不阻塞模块完成初始化，避免内联动态导入的HEIC命名空间处于暂时未初始化状态。
void verifyConversion({host,add,getState:()=>({files,results,output,busy})});
