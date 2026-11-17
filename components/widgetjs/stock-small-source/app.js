import {connectHost} from '../../../sdk/host.js';
import {formatDecimal} from '../metal/metal-model.js';
import {chartGeometry} from '../gold-source/gold-model.js';
import {createStock,validateSelection,markets} from './stock-model.js';
import './style.css';
const $=id=>document.getElementById(id),host=await connectHost();let settings=await host.request('settings.get'),draft,disposed=false,searchGeneration=0,searchTimer;
validateSelection(settings);
function theme(){for(const key of ['background','foreground','primary','border'])document.documentElement.style.setProperty('--'+key,settings[key]);document.documentElement.style.setProperty('--radius',settings.radius+'px');}
const model=createStock(selection=>host.request('content.get',{source:'stock-minute',code:selection.code,market:selection.marketId}),render);
function draw(values,price,color){
  const canvas=$('chart'),context=canvas.getContext('2d');if(!context)return;const rect=canvas.getBoundingClientRect(),dpr=devicePixelRatio||1;canvas.width=Math.max(1,rect.width*dpr);canvas.height=Math.max(1,rect.height*dpr);context.scale(dpr,dpr);context.clearRect(0,0,rect.width,rect.height);
  if(!values?.length||!price||rect.height<=8||rect.width<=0)return;const {points}=chartGeometry(values,price,rect.width,rect.height);
  context.beginPath();points.forEach((point,index)=>index?context.lineTo(point.x,point.y):context.moveTo(point.x,point.y));context.strokeStyle=color;context.lineWidth=1.5;context.lineJoin='round';context.lineCap='round';context.stroke();
  const gradient=context.createLinearGradient(0,4,0,rect.height-4);gradient.addColorStop(0,color+'4d');gradient.addColorStop(1,color+'00');context.lineTo(rect.width,rect.height-4);context.lineTo(0,rect.height-4);context.closePath();context.fillStyle=gradient;context.fill();
}
function render(){
  if(disposed)return;const state=model.state,value=state.quote,change=value?.change;
  $('name').textContent=value?.name||settings.name;$('exchange').textContent=markets[settings.marketId];$('code').textContent=settings.code;
  const color=change?((change.sign>=0)===settings.upColorRed?'#f82842':'#5fc25d'):'#a6adc8';
  $('price').textContent=value?.price?formatDecimal(value.price,Math.min(value.decimal,2)):'—';$('price').title=value?.price?'原始分时末点：'+value.price:'';
  $('ratio').textContent=change?((change.sign>0?'↗ +':change.sign<0?'↘ ':'+')+change.percent+'%'):'—';$('ratio').style.color=color;$('ratio').title=change?'原始涨跌额：'+change.amount:'昨收或报价未提供';
  $('status').textContent=host.preview?'预览 · 不联网':state.error?'刷新失败：'+state.error+(value?'；保留上次分时':''):state.loading?'正在读取分时…':value&&!value.price?'暂无当日分时数据':'';
  $('time').textContent=(value?.times.length?'最后分时：'+value.times.at(-1):'等待来源分时')+(state.fetchedAt?' · 读取于'+new Date(state.fetchedAt).toLocaleTimeString('zh-CN',{hour12:false})+(Date.now()-state.fetchedAt>120000?'（超过2分钟）':''):'');
  if(value?.sourceStatus!=null)$('time').textContent+=' · 源状态 '+value.sourceStatus;
  $('refresh').disabled=host.preview||state.loading;draw(value?.values,value?.price,color);
}
function selected(){ $('selected').textContent='待保存证券：'+draft.name+' · '+markets[draft.marketId]+' '+draft.code; }
$('settings').onclick=()=>{draft=validateSelection(settings);selected();$('results').replaceChildren();$('search').value='';$('search-status').textContent='';$('error').textContent='';for(const key of ['background','foreground','primary','border'])$(key).value=settings[key];$('radius').value=settings.radius;$('upColorRed').checked=settings.upColorRed;if(!$('config').open)$('config').showModal();};
$('close').onclick=()=>{searchGeneration++;clearTimeout(searchTimer);$('config').close();};$('config').addEventListener('close',()=>{searchGeneration++;clearTimeout(searchTimer);});
async function search(){
  clearTimeout(searchTimer);if(host.preview)return;const query=$('search').value.trim(),request=++searchGeneration;$('results').replaceChildren();if(!query){$('search-status').textContent='';return;}$('search-status').textContent='正在搜索…';
  try{const response=await host.request('content.get',{source:'stock-search',query});if(disposed||request!==searchGeneration||!$('config').open)return;if(!Array.isArray(response.data))throw new Error('搜索列表无效');
    for(const candidate of response.data){const selection=validateSelection(candidate),button=document.createElement('button');button.type='button';const name=document.createElement('span');name.textContent=selection.name;const identity=document.createElement('small');identity.textContent=markets[selection.marketId]+' '+selection.code;button.append(name,identity);button.onclick=()=>{draft=selection;selected();};$('results').append(button);}
    $('search-status').textContent=response.data.length?'点击选择证券，再保存设置':'没有匹配证券';
  }catch(error){if(!disposed&&request===searchGeneration)$('search-status').textContent='搜索失败：'+String(error?.message||error);}
}
$('search-button').onclick=()=>void search();$('search').addEventListener('input',()=>{searchGeneration++;clearTimeout(searchTimer);searchTimer=setTimeout(()=>void search(),300);});$('search').addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();void search();}});
$('form').onsubmit=async event=>{event.preventDefault();if(host.preview)return;$('save').disabled=true;try{const next={...settings,...validateSelection(draft),radius:Number($('radius').value),upColorRed:$('upColorRed').checked};for(const key of ['background','foreground','primary','border'])next[key]=$(key).value;settings=await host.request('settings.update',next);theme();$('config').close();void model.select(settings);}catch(error){$('error').textContent='设置未保存：'+String(error?.message||error);}finally{$('save').disabled=host.preview;}};
for(const id of ['save','search','search-button'])$(id).disabled=host.preview;
$('refresh').onclick=()=>void model.refresh();theme();render();const observer=new ResizeObserver(render);observer.observe($('chart'));await host.ready();if(!host.preview)void model.select(settings);
const refresh=setInterval(()=>{if(!host.preview&&!model.state.loading)void model.refresh();},60000),age=setInterval(render,15000);
window.addEventListener('pagehide',()=>{disposed=true;clearInterval(refresh);clearInterval(age);clearTimeout(searchTimer);searchGeneration++;observer.disconnect();model.dispose();},{once:true});
