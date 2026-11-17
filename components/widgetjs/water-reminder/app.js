import { connectHost } from './host.js';
import { createWater,waveProgress } from './water-model.js';
import { WaveBall } from './wave-ball.js';
import { verifyWater } from './verify.js';
const $=id=>document.getElementById(id),host=await connectHost();let settings=await host.request('settings.get'),disposed=false,frame=0,targetProgress=0,fromProgress=0,displayProgress=0,transitionAt=performance.now(),saving=false;
const keys=['enableReminder','interval','targetCup','background','color','primary'];const ball=new WaveBall(),ctx=$('canvas').getContext('2d');
const model=createWater((method,args)=>host.request(method,args),render);
function safeColor(value,fallback){return /^#[0-9a-f]{6}$/i.test(value)?value:fallback;}
function theme(){const root=document.documentElement;root.style.setProperty('--background',safeColor(settings.background,'#ffffff'));root.style.setProperty('--color',safeColor(settings.color,'#092239'));const color=safeColor(settings.primary,'#2596ff'),rgb=[1,3,5].map(i=>parseInt(color.slice(i,i+2),16));root.style.setProperty('--primary',color);ball.color=`rgba(${rgb.join(',')},.5)`;root.style.setProperty('--wave',ball.color);root.style.setProperty('--border',`rgba(${rgb.map(n=>Math.round((n+255)/2)).join(',')},.8)`);resize();}
function resize(){document.documentElement.style.fontSize=(innerWidth*16/155)+'px';const size=$('canvas').getBoundingClientRect().width;if(!ctx||size<=0)return;const ratio=Math.min(devicePixelRatio||1,3);$('canvas').width=Math.ceil(size*ratio);$('canvas').height=Math.ceil(size*ratio);ctx.setTransform(ratio,0,0,ratio,0,0);ball.size=size;ball.x=size/2;ball.y=size/2;ball.render(ctx);}
function render(){if(disposed)return;const value=model.state.snapshot,count=value?.count??null,target=value?.targetCup??settings.targetCup;const next=waveProgress(count,target)??0;if(next!==targetProgress){fromProgress=displayProgress;targetProgress=next;transitionAt=performance.now();}
  $('count').textContent=(count===null?'—':count)+'/'+target;$('daily').textContent='每天'+target+'杯水';
  const error=model.state.error||value?.lastError;$('status').textContent=host.preview?'预览 · 不启动提醒':model.state.pending?'杯数未确认，请重试':error?'读取或提醒失败：'+error:!value?'正在读取…':!value.enableReminder?'提醒已关闭':value.count>=value.targetCup?'今日目标已完成':value.delivery==='accepted'?'提醒已交给系统':value.delivery==='unknown'?'上次递送结果未确定':'';
  $('water').title=value?value.day+' · 每'+value.interval+'分钟提醒，隐藏组件后继续':'';$('retry').hidden=!error&&!model.state.pending;$('retry').disabled=model.state.busy;
  for(const key of ['add','reset'])$(key).disabled=host.preview||!value||model.state.busy||!!model.state.pending||saving;
  $('save').disabled=host.preview||model.state.busy||!!model.state.pending||saving;
}
function animate(now){if(disposed)return;const t=Math.min(1,(now-transitionAt)/300);displayProgress=fromProgress+(targetProgress-fromProgress)*(1-(1-t)**3);if(ctx){ctx.clearRect(0,0,ball.size,ball.size);ball.update(displayProgress);}frame=requestAnimationFrame(animate);}
$('add').onclick=()=>void model.change('add');$('reset').onclick=()=>void model.change('reset');$('retry').onclick=()=>void model.retry();
$('settings').onclick=()=>{for(const key of keys)if(key==='enableReminder')$(key).checked=settings[key];else $(key).value=settings[key];$('save-error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview||saving||model.state.busy||model.state.pending)return;saving=true;render();try{const next={...settings};for(const key of keys)next[key]=key==='enableReminder'?$(key).checked:['interval','targetCup'].includes(key)?Number($(key).value):$(key).value;settings=await host.request('settings.update',next);theme();await model.refresh();$('config').close();}catch(error){$('save-error').textContent='设置未保存：'+String(error?.message||error);}finally{saving=false;render();}};
const observer=new ResizeObserver(resize);observer.observe($('canvas'));theme();render();await host.ready();if(!host.preview)await model.refresh();frame=requestAnimationFrame(animate);const timer=setInterval(()=>{if(!host.preview&&!saving)void model.refresh();},5000);
window.addEventListener('pagehide',()=>{disposed=true;model.dispose();observer.disconnect();cancelAnimationFrame(frame);clearInterval(timer);},{once:true});
void verifyWater({host,model});
