import {connectHost} from '../../../sdk/host.js';
import {formatDecimal} from '../metal/metal-model.js';
import {drawCoinChart} from './coin-chart.js';
import {createCoin,coins,isOld,validateSettings} from './coin-model.js';
import './style.css';
import {verifySmall} from '../coin-common/verify.js';
const $=id=>document.getElementById(id),host=await connectHost(),fallback=new URL('./bitcoin.png',import.meta.url).href;
let settings=validateSettings(await host.request('settings.get')),disposed=false,refreshTimer;
const model=createCoin(instrument=>host.request('content.get',{source:'coin-index',instrument}),render);
function theme(){for(const key of ['background','foreground','primary','border'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--radius',settings.radius+'px');}
function logo(){const coin=coins.find(coin=>coin.type===settings.coin);$('logo').onerror=()=>{if($('logo').src!==fallback)$('logo').src=fallback;};$('logo').src=host.preview?fallback:coin.logo;}
function draw(color){drawCoinChart($('chart'),model.state.points.map(point=>point.price),color);}
function render(){if(disposed)return;const state=model.state,value=state.quote,coin=coins.find(coin=>coin.type===settings.coin),change=value?.change;
  $('name').textContent=coin.name;$('currency').textContent=coin.ccy;$('identity').textContent='OKX · '+settings.coin+' · 指数价 Index · USD';
  const color=change?((change.sign>=0)===settings.upColorRed?'#f82842':'#5fc25d'):'#a6adc8';$('price').textContent=value?'$'+formatDecimal(value.price):'—';$('price').style.color=color;$('price').title=value?'原始指数价：'+value.price:'';
  $('ratio').textContent=change?((change.sign>=0?'↗ +':'↘ ')+change.percent+'%'):'—';$('ratio').style.color=color;
  $('status').textContent=host.preview?'预览 · 不联网':state.error?'刷新失败：'+state.error+(value?'；保留上次指数':''):state.loading?(value?'正在后台更新…':'正在读取指数…'):value&&isOld(value,Date.now(),settings.refreshMs)?'旧指数，请查看来源时间':'';
  $('time').textContent=value?((state.error||isOld(value,Date.now(),settings.refreshMs)?'旧数据 · ':'')+new Date(value.timestamp).toLocaleString('zh-CN',{hour12:false})):'等待源时间';
  $('basis').textContent=value?.basis==='open24h'?'涨跌基准：24小时前开盘价':value?.basis==='sodUtc8'?'涨跌基准：UTC+8当日开盘价（原24小时基准缺失）':'参考基准未提供，不计算零涨跌';
  $('refresh').disabled=host.preview||state.loading;draw(color);
}
for(const coin of coins){const option=document.createElement('option');option.value=coin.type;option.textContent=coin.name+' · '+coin.ccy;$('coin').append(option);}
function schedule(){clearInterval(refreshTimer);refreshTimer=setInterval(()=>{if(!host.preview&&!model.state.loading)void model.refresh();},settings.refreshMs);}
$('settings').onclick=()=>{for(const key of ['coin','refreshMs','proxyMode','proxyHost','proxyPort','background','foreground','primary','border','radius'])$(key).value=settings[key];$('upColorRed').checked=settings.upColorRed;$('error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{const next={...settings,refreshMs:Number($('refreshMs').value),radius:Number($('radius').value),proxyPort:Number($('proxyPort').value),upColorRed:$('upColorRed').checked};for(const key of ['coin','proxyMode','proxyHost','background','foreground','primary','border'])next[key]=$(key).value;validateSettings(next);const networkChanged=['proxyMode','proxyHost','proxyPort'].some(key=>next[key]!==settings[key]);settings=await host.request('settings.update',next);if(networkChanged)model.reset();theme();logo();$('config').close();schedule();void model.select(settings.coin);}catch(error){$('error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
$('save').disabled=host.preview;$('refresh').onclick=()=>void model.refresh();theme();logo();render();
if(!host.preview){try{model.restore(await host.request('content.peek',{source:'coin-index',instrument:settings.coin}),settings.coin);}catch{console.warn('临时指数快照不可用，将读取来源');}}
const observer=new ResizeObserver(render);observer.observe($('chart'));await host.ready();if(!host.preview)void model.select(settings.coin);schedule();const age=setInterval(render,5000);
window.addEventListener('pagehide',()=>{disposed=true;clearInterval(refreshTimer);clearInterval(age);observer.disconnect();model.dispose();},{once:true});
void verifySmall({host,model,getState:()=>({settings,disposed}),isOld});
