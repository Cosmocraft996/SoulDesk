import { encodeBmp, encodeIco, outputDimensions, outputMime } from './conversion.js';

async function imageFromFile(file) {
  let blob=file;
  if (file.type==='image/heic'||file.type==='image/heif'||/\.hei[cf]$/i.test(file.name)) {
    try { const {default:heic2any}=await import('heic2any');const result=await heic2any({blob:file,toType:'image/jpeg'});blob=Array.isArray(result)?result[0]:result; }
    catch(error) { if(!(error?.code===1&&String(error.message).includes('already browser readable')))throw new Error('HEIC解码失败：'+String(error?.message||error)); }
  }
  const url=URL.createObjectURL(blob);
  try {
    return await new Promise((resolve,reject)=>{const image=new Image();image.onload=()=>resolve(image);image.onerror=()=>reject(new Error('图片无法解码'));image.src=url;});
  } finally { URL.revokeObjectURL(url); }
}
function toBlob(canvas,mime) {
  return new Promise((resolve,reject)=>canvas.toBlob(blob=>{
    if(!blob)reject(new Error('图片编码失败'));
    else if(blob.type!==mime)reject(new Error('当前系统不支持此输出格式，未将其他格式冒充该文件'));
    else resolve(blob);
  },mime,.9));
}
export async function convertImage(file,options) {
  const image=await imageFromFile(file),[width,height]=outputDimensions(image.naturalWidth,image.naturalHeight,options);
  const canvas=document.createElement('canvas');canvas.width=width;canvas.height=height;
  try {
    const context=canvas.getContext('2d');if(!context)throw new Error('无法创建绘图画布');
    if(['jpg','bmp'].includes(options.format)){context.fillStyle='#fff';context.fillRect(0,0,width,height);}
    context.drawImage(image,0,0,width,height);
    if(options.format==='bmp')return new Blob([encodeBmp(width,height,context.getImageData(0,0,width,height).data)],{type:'image/bmp'});
    if(options.format==='ico'){const png=new Uint8Array(await (await toBlob(canvas,'image/png')).arrayBuffer());return new Blob([encodeIco(width,height,png)],{type:'image/x-icon'});}
    return await toBlob(canvas,outputMime(options.format));
  } finally { canvas.width=1;canvas.height=1; }
}
