import { connectHost } from '../../../sdk/host.js';
import { formatDecimal } from '../metal/metal-model.js';
import { createGold, goldChange, chartMatchesQuote, quoteIsOld } from './gold-model.js';
import { drawGoldChart } from './gold-chart.js';
import './style.css';
const $=id=>document.getElementById(id),host=await connectHost();let settings=await host.request('settings.get'),disposed=false;
function theme(){for(const key of ['background','foreground','primary','border'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--radius',settings.radius+'px');}
const model=createGold(()=>host.request('content.get',{source:'gold'}),()=>host.request('content.get',{source:'gold-trends'}),render);
function render(){
  if(disposed)return;const state=model.state,value=state.quote;
  $('refresh').disabled=host.preview||state.loading;
  $('status').textContent=host.preview?'预览 · 不联网':state.error?'报价刷新失败：'+state.error+(value?'；保留上次报价':''):state.loading?'正在刷新…':value&&quoteIsOld(value)?'旧报价，请结合来源时间查看':'';
  $('price').textContent=value?formatDecimal(value.latestPrice):'暂无有效报价';$('price').title=value?'原始最新价：'+value.latestPrice+' 元/克':'';
  const change=value?goldChange(value):null;$('change').textContent=change?(change.sign>0?'+':'')+change.percent+'%':'—';$('change').style.color=change?(change.sign>=0?'#e53935':'#43a047'):'inherit';$('change').title=change?'与昨收相比变动：'+change.amount+' 元/克':'昨收未提供，不计算涨跌';
  for(const [id,key]of [['previous','yesterdayClose'],['maximum','max'],['minimum','min']])$(id).textContent=value?.[key]!=null?formatDecimal(value[key]):'—';
  $('time').textContent=value?(quoteIsOld(value)||state.error?'旧数据 · ':'')+new Date(value.timestamp).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false}):'等待来源时间';
  const matching=chartMatchesQuote(state.series,value);
  const lastPoint=state.series?.timestamps.at(-1),chartTime=matching?'分时截至 '+state.series.times.at(-1)+(Date.now()-lastPoint>300000?' · 旧分时':''):'';
  $('chart-status').textContent=host.preview?'':state.chartError?'分时刷新失败：'+state.chartError+(chartTime?'；'+chartTime:''):state.series?.values.length&&!matching?'分时与报价日期不符，暂不混合展示':chartTime||(!state.series?.values.length&&!state.loading?'暂无当日分时数据':'');
  drawGoldChart($('chart'),matching?state.series.values:[],value?.latestPrice);
}
function fill(){for(const key of ['background','foreground','primary','border'])$(key).value=settings[key];$('radius').value=settings.radius;}
$('settings').onclick=()=>{fill();$('save-error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{const next={...settings,radius:Number($('radius').value)};for(const key of ['background','foreground','primary','border'])next[key]=$(key).value;settings=await host.request('settings.update',next);theme();render();$('config').close();}catch(error){$('save-error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
$('refresh').onclick=()=>void model.refresh();theme();render();$('save').disabled=host.preview;
const observer=new ResizeObserver(render);observer.observe($('chart'));
await host.ready();if(!host.preview)void model.refresh();
const timer=setInterval(()=>{if(!host.preview&&!model.state.loading)void model.refresh();},5000),ageTimer=setInterval(render,5000);
window.addEventListener('pagehide',()=>{disposed=true;clearInterval(timer);clearInterval(ageTimer);observer.disconnect();model.dispose();},{once:true});
