import { connectHost } from '../host.js';
import { createMetrics } from '../hardware-common/metrics.js';
import { energy } from './energy-model.js';
import { verifyHardware } from '../hardware-common/verify.js';
const $=id=>document.getElementById(id),host=await connectHost();let settings=await host.request('settings.get'),disposed=false;
const model=createMetrics(()=>host.request('system.metrics'),render);
function theme(){document.documentElement.style.setProperty('--font-size',settings.fontSize+'px');document.documentElement.style.setProperty('--radius',settings.radius+'px');$('productor-label').textContent=settings.productor;$('spec-label').textContent=settings.spec;}
function render(){if(disposed)return;const view=energy(model.state.snapshot),error=model.state.error;
  $('level').textContent=view.level===null?'—':String(view.level);$('cpu').textContent=view.cpuText;$('memory').textContent=view.memoryText;
  document.documentElement.style.setProperty('--current',view.color);document.documentElement.style.setProperty('--top',view.top+'px');$('current').style.opacity=view.level===null?'.5':error?'.65':'1';
  $('status').textContent=host.preview?'预览 · 不读取硬件':error?'旧数据 · 读取失败：'+error:view.status;
  $('current').title=view.score===null?view.status:'加权占用率 '+view.score.toFixed(2)+'%'+(model.state.snapshot?' · 采样于 '+new Date(model.state.snapshot.sampledAt).toLocaleTimeString('zh-CN',{hour12:false}):'');
}
$('settings').onclick=()=>{for(const key of ['productor','spec','fontSize','radius'])$(key).value=settings[key];$('save-error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{
  settings=await host.request('settings.update',{...settings,productor:$('productor').value,spec:$('spec').value,fontSize:Number($('fontSize').value),radius:Number($('radius').value)});theme();render();$('config').close();
}catch(error){$('save-error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
theme();render();$('save').disabled=host.preview;await host.ready();if(!host.preview)void model.refresh();
await verifyHardware({host,kind:'energy-label',model,getSettings:()=>settings,expectedView:energy});
const timer=setInterval(()=>{render();if(!host.preview)void model.refresh();},3000);
window.addEventListener('pagehide',()=>{disposed=true;model.dispose();clearInterval(timer);},{once:true});
