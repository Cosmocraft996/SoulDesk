import { connectHost } from './host.js';
import { DEFAULT_PICTURE, validatePicture, stickerSize, loadPicture } from './model.js';
import { verifySticker } from './verify.js';
const $=id=>document.getElementById(id),host=await connectHost(),stored=await host.request('storage.get');
if (stored.image !== undefined && typeof stored.image !== 'string') throw new Error('已保存图片格式无效，请保留原数据后检查');
let source=stored.image || DEFAULT_PICTURE,currentImage,pendingSource,disposed=false,busy=false;
function setBusy(value) { busy=value;for (const id of ['local','save','retry']) $(id).disabled=value || host.preview; }
function message(value) { $('status').textContent=value;$('message').textContent=value; }
function resize() { if(currentImage){const [width,height]=stickerSize(currentImage.naturalWidth,currentImage.naturalHeight,innerWidth,innerHeight);currentImage.style.width=width+'px';currentImage.style.height=height+'px';} }
async function apply(value,persist) {
  if(host.preview)return;
  setBusy(true);if(persist)pendingSource=value;message('正在加载图片…');
  try {
    validatePicture(value);const image=await loadPicture(value);
    if (disposed) return;
    if(persist) { message('正在保存图片，请稍候…');await host.request('storage.set',{image:value}); }
    if (disposed) return;
    source=value;currentImage=image;image.alt='大头贴照片';$('sticker').replaceChildren(image);resize();
    pendingSource=undefined;$('retry').hidden=true;$('url').value=source.startsWith('https:')?source:'';
    message(persist?'图片已保存':'');
  } catch(error) { if(!disposed){message((persist?'未保存：':'图片未加载：')+String(error?.message||error));$('retry').hidden=!pendingSource;} }
  finally { if(!disposed)setBusy(false); }
}
$('settings').onclick=()=>{if(!$('config').open)$('config').showModal();};$('close').onclick=()=>$('config').close();
$('form').onsubmit=event=>{event.preventDefault();if(!busy)void apply($('url').value.trim(),true);};
$('retry').onclick=()=>{if(!busy&&pendingSource)void apply(pendingSource,true);};
$('local').onclick=async()=>{
  if(busy)return;setBusy(true);
  try {
    const files=await host.request('documents.open',{fileName:'图片.png',extensions:['jpg','jpeg','gif','png','webp']});
    if(files?.length) { const file=files[0];await apply('data:'+file.mime+';base64,'+file.data,true); }
    else message('已取消，保留原图');
  } catch(error) { message('本地图片未更换：'+String(error?.message||error)); }
  finally { if(!disposed)setBusy(false); }
};
$('url').value=source.startsWith('https:')?source:'';setBusy(false);
await host.ready();
if(host.preview)message('预览 · 不读取本地或远程图片');
else if(new URLSearchParams(location.search).get('verify')==='widgetjs-photo')void verifySticker({host,apply,getImage:()=>currentImage});
else void apply(source,false);
window.addEventListener('resize',resize);window.addEventListener('pagehide',()=>{disposed=true;window.removeEventListener('resize',resize);},{once:true});
