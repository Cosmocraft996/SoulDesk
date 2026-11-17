import { connectHost } from '../../../sdk/host.js';
import { formatDecimal, signOf } from '../metal/metal-model.js';
import { createIndices } from './index-model.js';
import './style.css';
const $=id=>document.getElementById(id),host=await connectHost();let settings=await host.request('settings.get'),disposed=false;
const model=createIndices(code=>host.request('content.get',{source:'a-index',code}),render);
function render(){
  if(disposed)return;$('indices').replaceChildren();$('refresh').disabled=host.preview||model.state.loading;
  for(const item of model.state.rows){
    const value=item.quote,row=document.createElement('article');row.className='index-row';
    const label=document.createElement('div');label.className='index-label';const name=document.createElement('strong');name.textContent=value?.name||item.name;
    const identity=document.createElement('div');identity.className='index-code';const exchange=document.createElement('span');exchange.className='index-exchange';exchange.textContent=item.exchange;const code=document.createElement('span');code.textContent=item.code;identity.append(exchange,code);label.append(name,identity);
    const price=document.createElement('div');price.className='index-price';price.textContent=value?formatDecimal(value.price):'--';price.title=value?'原始指数值：'+value.price:'';
    const change=document.createElement('div');change.className='index-change';
    if(value){const sign=signOf(value.ratio);change.textContent=(sign>=0?'+':'')+formatDecimal(value.ratio)+'%';change.style.backgroundColor=(sign>=0)===settings.upColorRed?'#f82842':'#5fc25d';change.title='原始涨跌幅：'+value.ratio+'%';}else change.textContent=item.loading?'加载中':'--';
    const meta=document.createElement('div');meta.className='index-meta'+(item.error?' error':'');
    meta.textContent=host.preview?'预览，不联网':item.error?('刷新失败：'+item.error+(value?'；保留上次值':'')):value?(value.stockStatusInfo+' · '+(value.sourceTimeText?'源时间文本：'+value.sourceTimeText:'源未提供报价时间')):'等待有效报价';
    if(item.fetchedAt)meta.textContent+=' · 读取于'+new Date(item.fetchedAt).toLocaleTimeString('zh-CN',{hour12:false})+(Date.now()-item.fetchedAt>120000?'（超过2分钟）':'');
    row.append(label,price,change,meta);$('indices').append(row);
  }
  $('source-status').textContent='固定三大指数 · 60秒刷新 · 读取时间不代表报价时间，不参与币安警报';
}
$('settings').onclick=()=>{$('upColorRed').checked=settings.upColorRed;$('error').textContent='';if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{settings=await host.request('settings.update',{upColorRed:$('upColorRed').checked});render();$('config').close();}catch(error){$('error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
$('refresh').onclick=()=>void model.refresh();$('save').disabled=host.preview;render();await host.ready();if(!host.preview)void model.refresh();
const refresh=setInterval(()=>{if(!host.preview&&!model.state.loading)void model.refresh();},60000),age=setInterval(render,15000);
window.addEventListener('pagehide',()=>{disposed=true;clearInterval(refresh);clearInterval(age);model.dispose();},{once:true});
