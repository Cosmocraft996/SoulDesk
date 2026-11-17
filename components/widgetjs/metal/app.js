import { connectHost } from './host.js';
import { createMetals, formatDecimal, signOf, quoteIsOld } from './metal-model.js';
const $=id=>document.getElementById(id),host=await connectHost();
let settings=await host.request('settings.get'),disposed=false;
function theme(){for(const key of ['background','foreground','primary','border'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--radius',settings.radius+'px');}
const model=createMetals(()=>host.request('content.get',{source:'metal'}),render);
function render(){
  if(disposed)return;const state=model.state;
  $('status').textContent=host.preview?'预览 · 不联网':state.error?'刷新失败：'+state.error+(state.quotes.length?'；以下为上次报价':''):state.loading?'正在读取报价…':!state.quotes.length?'暂无有效报价':'';
  $('refresh').disabled=host.preview||state.loading;$('metals').replaceChildren();
  for(const quote of state.quotes){
    const row=document.createElement('article');row.className='metal-item';
    const label=document.createElement('div');label.className='metal-label';
    const name=document.createElement('strong');name.textContent=quote.name;
    const code=document.createElement('span');code.className='metal-code';code.dataset.code=quote.code;code.textContent=quote.code.replace('USD','');label.append(name,code);
    const price=document.createElement('div');price.className='metal-price';price.textContent='$'+formatDecimal(quote.price);price.title='原始中间价：'+quote.price+' USD';
    const sign=signOf(quote.changePercent),change=document.createElement('div');change.className='metal-change';change.dataset.sign=String(sign);change.textContent=(sign>0?'+':'')+formatDecimal(quote.changePercent)+'%';change.title='原始涨跌百分比：'+quote.changePercent+'%';change.style.backgroundColor=(sign>0)===settings.upColorRed?'#f82842':'#5fc25d';
    const meta=document.createElement('div'),old=quoteIsOld(quote)||Boolean(state.error);meta.className='metal-meta'+(old?' old':'');meta.textContent=quote.code+' · '+(quote.dataSource||'上游未提供')+' · '+(quote.exchange||'交易所未提供')+' · '+quote.marketState+' · '+(old?'旧数据 · ':'')+new Date(quote.updatedAt).toLocaleString('zh-CN',{hour12:false});
    row.append(label,price,change,meta);$('metals').append(row);
  }
}
function fill(){for(const key of ['background','foreground','primary','border'])$(key).value=settings[key];$('radius').value=settings.radius;$('upColorRed').checked=settings.upColorRed;}
$('settings').onclick=()=>{fill();$('save-error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{
  event.preventDefault();if(host.preview)return;$('save').disabled=true;
  try{const next={...settings,radius:Number($('radius').value),upColorRed:$('upColorRed').checked};for(const key of ['background','foreground','primary','border'])next[key]=$(key).value;
    settings=await host.request('settings.update',next);theme();render();$('config').close();
  }catch(error){$('save-error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}
};
$('refresh').onclick=()=>void model.refresh();theme();render();$('save').disabled=host.preview;
await host.ready();if(!host.preview)void model.refresh();
const timer=setInterval(()=>{if(!host.preview)void model.refresh();},30000),ageTimer=setInterval(render,10000);
window.addEventListener('pagehide',()=>{disposed=true;clearInterval(timer);clearInterval(ageTimer);model.dispose();},{once:true});
