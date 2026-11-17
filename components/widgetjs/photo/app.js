import { connectHost } from './host.js';
import { createCarousel, loadPicture } from './model.js';
import { verifyPhoto } from './verify.js';
const $ = id => document.getElementById(id), host = await connectHost();
let settings = await host.request('settings.get'), directory = null, timer, currentImage, disposed = false, directoryRequest = 0;
const model = createCarousel(async(name,revision)=>{
  const file = await host.request('media.read',{name,revision});
  return loadPicture('data:'+file.mime+';base64,'+file.data);
},state=>{
  if (state.image !== currentImage) {
    const previous = currentImage; currentImage = state.image;
    if (currentImage) { currentImage.className='photo-item';currentImage.alt=state.name;$('photo').prepend(currentImage); }
    if (previous) {
      if (currentImage && previous.animate) { previous.style.zIndex='1001'; const animation=previous.animate([{opacity:1},{opacity:0}],{duration:800,fill:'forwards'});animation.addEventListener('finish',()=>previous.remove(),{once:true}); }
      else previous.remove();
    }
  }
  $('guide').hidden=Boolean(state.image);$('guide').textContent=state.loading?'正在加载照片…':directory?'文件夹内没有可显示的图片，点击设置':'点击设置图片文件夹';
  $('previous').hidden=$('next').hidden=!state.files.length;
  $('status').textContent=state.error?('图片未更新：'+state.error):host.preview?'预览 · 不读取本地文件':state.loading?'正在加载…':'';
});
function schedule(extra=0) {
  clearTimeout(timer);
  if (disposed || host.preview || !model.state.files.length) return;
  timer=setTimeout(async()=>{ await model.move(1);schedule(); },settings.duration*1000+extra);
}
function fillForm() { $('duration').value=settings.duration;$('random').checked=settings.random;$('radius').value=settings.radius;document.documentElement.style.setProperty('--widget-border-radius',settings.radius+'px'); }
async function applyDirectory(value) {
  directory=value;$('folder').textContent=directory?'文件夹：'+directory.name:'尚未选择文件夹';
  await model.load(directory || {revision:'',files:[]},settings.random);schedule();
}
function showError(error) { const message=String(error?.message||error);$('status').textContent=message;$('config-status').textContent=message; }
async function refresh() {
  if (host.preview) return;
  const request=++directoryRequest;
  try { const value=await host.request('media.list');if(request!==directoryRequest||disposed)return;await applyDirectory(value);$('config-status').textContent='已刷新文件列表'; }
  catch(error) { if(request===directoryRequest&&!disposed)showError(error); }
}
function openConfig() { if (!$('config').open) $('config').showModal(); }
$('settings').onclick=$('guide').onclick=openConfig;
$('close').onclick=()=>$('config').close();
$('previous').onclick=async()=>{await model.move(-1);schedule();};$('next').onclick=async()=>{await model.move(1);schedule();};
$('photo').onmousemove=()=>schedule(3000);
$('refresh').onclick=refresh;
$('choose').onclick=async()=>{
  if(host.preview)return;const request=++directoryRequest;
  $('choose').disabled=true;
  try { const selected=await host.request('documents.open',{directory:true});if(request!==directoryRequest||disposed)return;if (selected) { await applyDirectory(selected);$('config-status').textContent='目录已保存'; } else $('config-status').textContent='已取消，保留原目录'; }
  catch(error) { if(request===directoryRequest&&!disposed)showError(error); }
  finally { $('choose').disabled=false; }
};
$('clear').onclick=async()=>{
  if(host.preview)return;const request=++directoryRequest;
  try { await host.request('media.clear');if(request!==directoryRequest||disposed)return;await applyDirectory(null);$('config-status').textContent='已清除本实例目录授权，原照片保留'; }
  catch(error) { if(request===directoryRequest&&!disposed)showError(error); }
};
$('form').onsubmit=async event=>{
  event.preventDefault();if(host.preview)return;const button=$('form').querySelector('button[type=submit]');button.disabled=true;
  try {
    const next={...settings,duration:Number($('duration').value),random:$('random').checked,radius:Number($('radius').value)};
    settings=await host.request('settings.update',next);fillForm();
    if (directory) await applyDirectory(directory); else schedule();
    $('config-status').textContent='设置已保存';
  } catch(error) { fillForm();showError(error); }
  finally { button.disabled=false; }
};
fillForm();await applyDirectory(null);
for (const id of ['choose','refresh','clear']) $(id).disabled=host.preview;
$('form').querySelector('button[type=submit]').disabled=host.preview;
await host.ready();
if (!host.preview) void refresh();
void verifyPhoto({host,getState:()=>({directory,model:model.state}),refresh,stop:()=>clearTimeout(timer)});
window.addEventListener('pagehide',()=>{disposed=true;clearTimeout(timer);model.dispose();},{once:true});
