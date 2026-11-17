import {connectHost} from '../../../sdk/host.js';
import {formatDecimal} from '../metal/metal-model.js';
import {coins,fallbacks,createFeed,createDisplay,validateSettings,quoteIsOld} from './coin-model.js';
import './style.css';
import {verifyMedium} from '../coin-common/verify.js';
const $=id=>document.getElementById(id),host=await connectHost();
let settings=validateSettings(await host.request('settings.get')),disposed=false,displayTimer;
const rows=new Map(),display=createDisplay(),model=createFeed(()=>host.request('content.live',{source:'coin-currencies'}),render);
function localIcon(coin){return new URL('./'+(fallbacks[coin.ccy]||'bitcoin.png'),location.href).href;}
for(const coin of coins){const row=document.createElement('section'),logo=document.createElement('img'),label=document.createElement('div'),symbol=document.createElement('strong'),name=document.createElement('small'),value=document.createElement('div'),price=document.createElement('span'),time=document.createElement('time'),change=document.createElement('span'),meta=document.createElement('div');
  row.className='ticker';logo.className='logo';logo.alt=coin.name;logo.onerror=()=>{const fallback=localIcon(coin);if(logo.src!==fallback)logo.src=fallback;};logo.src=host.preview?localIcon(coin):coin.logo;label.className='name';symbol.textContent=coin.ccy;name.textContent=coin.name;label.append(symbol,name);value.className='value';price.className='price';price.textContent='—';time.className='time';value.append(price,time);change.className='change';change.textContent='—';meta.className='meta';row.append(logo,label,value,change,meta);$('list').append(row);rows.set(coin.ccy,{row,price,time,change,meta});
}
function theme(){for(const key of ['background','foreground','primary','border'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--radius',settings.radius+'px');}
function render(){if(disposed)return;const {snapshot,error,loading}=model.state,now=Date.now(),shown=display.update(snapshot,settings.refreshMs,now);
  for(const coin of coins){const {row,price,time,change,meta}=rows.get(coin.ccy),quote=shown.get(coin.ccy),old=Boolean(quote&&(error||quoteIsOld(snapshot,quote,now,settings.refreshMs))),ratio=quote?.change;
    price.textContent=quote?'$'+formatDecimal(quote.price):'—';price.title=quote?'原始网页报价：'+quote.price:'';
    change.textContent=ratio?((ratio.sign>=0?'+':'')+ratio.percent+'%'):'—';change.style.background=ratio?((ratio.sign>=0)===settings.upColorRed?'#f82842':'#5fc25d'):'#ffffff20';change.title=ratio?'相对24小时前开盘价：'+quote.reference:'24小时参考价格未提供';
    time.textContent=quote?(old?'旧 · ':'')+new Date(quote.timestamp).toLocaleTimeString('zh-CN',{hour12:false}):'等待源时间';
    time.title=quote?'来源：'+new Date(quote.timestamp).toLocaleString('zh-CN',{hour12:false})+'\n本机接收：'+new Date(quote.receivedAt).toLocaleString('zh-CN',{hour12:false}):'';
    row.classList.toggle('stale',old);meta.textContent=snapshot?.currencyErrors[coin.ccy]||(!quote?(host.preview?'':'等待来源提供该币种报价'):!ratio?'24小时基准未提供，保留价格':old?'保留上次报价，请查看来源时间':'');
  }
  $('connection').textContent=host.preview?'主流虚拟币 · 预览':!snapshot?'正在准备原生连接…':snapshot.connection==='connected'?'主流虚拟币 · 已连接':snapshot.connection==='retrying'?'主流虚拟币 · 正在重连':'主流虚拟币 · 正在连接';
  $('status').textContent=host.preview?'预览不联网':error?'读取失败：'+error:snapshot?.error||'';$('refresh').disabled=host.preview||loading;
}
function scheduleDisplay(){clearInterval(displayTimer);displayTimer=setInterval(render,settings.refreshMs);}
$('settings').onclick=()=>{for(const key of ['refreshMs','proxyMode','proxyHost','proxyPort','background','foreground','primary','border','radius'])$(key).value=settings[key];$('upColorRed').checked=settings.upColorRed;$('error').textContent='';if(!$('config').open)$('config').showModal();};
$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{const next={...settings,refreshMs:Number($('refreshMs').value),proxyPort:Number($('proxyPort').value),radius:Number($('radius').value),upColorRed:$('upColorRed').checked};for(const key of ['proxyMode','proxyHost','background','foreground','primary','border'])next[key]=$(key).value;validateSettings(next);const networkChanged=['proxyMode','proxyHost','proxyPort'].some(key=>next[key]!==settings[key]);settings=await host.request('settings.update',next);theme();display.reset();if(networkChanged)model.reset();$('config').close();scheduleDisplay();await model.refresh();}catch(error){$('error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
$('refresh').onclick=async()=>{await model.refresh();display.reset();render();};$('save').disabled=host.preview;
theme();render();scheduleDisplay();if(!host.preview)await model.refresh();await host.ready();
const poll=host.preview?null:setInterval(()=>{if(!model.state.loading)void model.refresh();},3000),age=setInterval(render,5000);
window.addEventListener('pagehide',()=>{disposed=true;clearInterval(poll);clearInterval(age);clearInterval(displayTimer);model.dispose();display.reset();},{once:true});
void verifyMedium({host,model,display,rows,getState:()=>({settings,disposed}),quoteIsOld});
