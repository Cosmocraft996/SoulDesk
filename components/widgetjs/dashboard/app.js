import { connectHost } from '../host.js';
import { createMetrics,gauge } from './gauge-model.js';
import { verifyHardware } from '../hardware-common/verify.js';
const $=id=>document.getElementById(id),host=await connectHost();
let settings=await host.request('settings.get'),disposed=false;
const model=createMetrics(()=>host.request('system.metrics'),render);
function theme(){for(const key of ['background','color'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--radius',settings.radius+'px');}
function render(){if(disposed)return;const view=gauge(model.state.snapshot,settings.monitorType),error=model.state.error;
  $('value').textContent=view.label;document.documentElement.style.setProperty('--rotate',view.angle+'deg');$('hand').style.opacity=view.valid&&!error?'1':'.35';
  $('status').textContent=host.preview?'预览 · 不读取硬件':error?'旧数据 · 读取失败：'+error:view.status;
  $('value').title=(settings.monitorType==='gpu'?'Windows 最繁忙GPU引擎':settings.monitorType==='cpu'?'Windows CPU总占用率':'Windows 物理内存占用率')+(model.state.snapshot?' · 采样于 '+new Date(model.state.snapshot.sampledAt).toLocaleTimeString('zh-CN',{hour12:false}):'');
}
$('settings').onclick=()=>{for(const key of ['monitorType','background','color','radius'])$(key).value=settings[key];$('save-error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{
  const next={...settings,monitorType:$('monitorType').value,background:$('background').value,color:$('color').value,radius:Number($('radius').value)};
  if(!['cpu','memory','gpu'].includes(next.monitorType))throw new Error('请选择有效监控类型');
  settings=await host.request('settings.update',next);theme();render();$('config').close();
}catch(error){$('save-error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
theme();render();$('save').disabled=host.preview;await host.ready();if(!host.preview)void model.refresh();
await verifyHardware({host,kind:'dashboard',model,getSettings:()=>settings,expectedView:gauge});
const timer=setInterval(()=>{render();if(!host.preview)void model.refresh();},3000);
window.addEventListener('pagehide',()=>{disposed=true;model.dispose();clearInterval(timer);},{once:true});
