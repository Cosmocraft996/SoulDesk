import { connectHost } from '../host.js';
import { createMetrics } from '../hardware-common/metrics.js';
import { panel } from './panel-model.js';
import { verifyHardware } from '../hardware-common/verify.js';
const $=id=>document.getElementById(id),host=await connectHost();let settings=await host.request('settings.get'),disposed=false,frame=0;
const rings=new Map(),model=createMetrics(()=>host.request('system.metrics'),render);
function theme(){for(const key of ['background','color','divider'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--font-size',settings.fontSize+'px');document.documentElement.style.setProperty('--radius',settings.radius+'px');}
function draw(canvas,value){const ctx=canvas.getContext('2d');if(!ctx)return;const start=Math.PI/12,full=Math.PI*11/6;
  ctx.clearRect(0,0,100,100);ctx.save();ctx.translate(50,50);ctx.rotate(Math.PI/2);ctx.lineWidth=12;ctx.lineCap='round';ctx.beginPath();ctx.arc(0,0,40,start,start+full);ctx.strokeStyle='rgba(222,222,222,.4)';ctx.stroke();
  if(value!==null&&value>0){const gradient=ctx.createConicGradient(0,0,0);for(const [at,color]of[[0,'#2cd228'],[.3,'#2cd228'],[.6,'#ffaf42'],[.8,'#ffaf42'],[.9,'#ff682d'],[1,'#ff492d']])gradient.addColorStop(at,color);ctx.beginPath();ctx.arc(0,0,40,start,start+full*value/100);ctx.strokeStyle=gradient;ctx.stroke();}ctx.restore();
}
function animate(at){frame=0;let moving=false;for(const [key,ring]of rings){const time=Math.min(1,Math.max(0,(at-ring.at)/1000)),ease=time<.5?4*time**3:1-(-2*time+2)**3/2;ring.current=ring.target===null?null:ring.from+(ring.target-ring.from)*ease;draw($(key+'-canvas'),ring.current);if(time<1&&ring.target!==null)moving=true;}if(moving&&!disposed)frame=requestAnimationFrame(animate);}
function render(){if(disposed)return;const view=panel(model.state.snapshot),error=model.state.error;
  for(const key of ['cpu','memory','gpu']){$(key+'-value').textContent=view[key].label;$(key+'-subtitle').textContent=view[key].subtitle;$(key+'-column').title=view[key].detail;const previous=rings.get(key);if(!previous||previous.target!==view[key].percent)rings.set(key,{from:previous?.current??0,current:previous?.current??null,target:view[key].percent,at:performance.now()});}
  $('download').textContent='↓ '+view.network.download;$('upload').textContent='↑ '+view.network.upload;$('received').textContent=view.network.received;$('sent').textContent=view.network.sent;$('network-column').title=view.network.detail;
  $('status').textContent=host.preview?'预览 · 不读取硬件':error?(model.state.snapshot?'旧数据 · ':'')+'读取失败：'+error:view.status;$('status').title=$('status').textContent+(model.state.snapshot?' · 采样于 '+new Date(model.state.snapshot.sampledAt).toLocaleTimeString('zh-CN',{hour12:false}):'');
  if(!frame)frame=requestAnimationFrame(animate);
}
$('settings').onclick=()=>{for(const key of ['fontSize','background','color','divider','radius'])$(key).value=settings[key];$('save-error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{
  const next={...settings,fontSize:Number($('fontSize').value),radius:Number($('radius').value)};for(const key of ['background','color','divider'])next[key]=$(key).value;settings=await host.request('settings.update',next);theme();render();$('config').close();
}catch(error){$('save-error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
theme();render();$('save').disabled=host.preview;await host.ready();if(!host.preview)void model.refresh();await verifyHardware({host,kind:'base-panel',model,getSettings:()=>settings,expectedView:panel});const timer=setInterval(()=>{render();if(!host.preview)void model.refresh();},3000);
window.addEventListener('pagehide',()=>{disposed=true;model.dispose();clearInterval(timer);cancelAnimationFrame(frame);},{once:true});
