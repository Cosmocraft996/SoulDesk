const formats={jpg:'image/jpeg',png:'image/png',webp:'image/webp',bmp:'image/bmp',ico:'image/x-icon'};
export function validateOptions(options) {
  if (!options || !Object.hasOwn(formats,options.format) || typeof options.keepAspectRatio!=='boolean' || !['width','height'].every(key=>Number.isInteger(options[key])&&options[key]>=0&&options[key]<=16384)) throw new Error('转换设置无效，尺寸需为0～16384的整数，0表示原始尺寸');
  return options;
}
export function outputDimensions(originalWidth,originalHeight,options) {
  validateOptions(options);
  if (![originalWidth,originalHeight].every(value=>Number.isFinite(value)&&value>0)) throw new Error('原始图片尺寸无效');
  let width=options.width||originalWidth,height=options.height||originalHeight;
  if (options.keepAspectRatio && Boolean(options.width)!==Boolean(options.height)) {
    if(options.width)height=width/originalWidth*originalHeight;else width=height/originalHeight*originalWidth;
  }
  if(options.format==='ico') {
    if (!options.width&&!options.height) { const ratio=Math.min(1,256/width,256/height);width*=ratio;height*=ratio; }
    if(width>256||height>256)throw new Error('ICO尺寸最多256×256，请调小尺寸或使用原始尺寸自动缩小');
  }
  width=Math.max(1,Math.round(width));height=Math.max(1,Math.round(height));
  if(width>16384||height>16384||width*height>16*1024*1024)throw new Error('输出画布超过16384边长或1600万像素上限，请缩小图片');
  return [width,height];
}
export function encodeBmp(width,height,pixels) {
  if(!Number.isInteger(width)||!Number.isInteger(height)||width<1||height<1||width*height>16*1024*1024||pixels.length!==width*height*4)throw new Error('BMP像素尺寸无效');
  const stride=Math.ceil(width*3/4)*4,bytes=new Uint8Array(54+stride*height),view=new DataView(bytes.buffer);
  view.setUint16(0,0x4d42,true);view.setUint32(2,bytes.length,true);view.setUint32(10,54,true);view.setUint32(14,40,true);
  view.setInt32(18,width,true);view.setInt32(22,-height,true);view.setUint16(26,1,true);view.setUint16(28,24,true);view.setUint32(34,stride*height,true);
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){const input=(y*width+x)*4,output=54+y*stride+x*3;bytes[output]=pixels[input+2];bytes[output+1]=pixels[input+1];bytes[output+2]=pixels[input];}
  return bytes;
}
export function encodeIco(width,height,png) {
  if(!Number.isInteger(width)||!Number.isInteger(height)||width<1||height<1||width>256||height>256||png.length<8||![137,80,78,71,13,10,26,10].every((value,index)=>png[index]===value))throw new Error('ICO尺寸或PNG内容无效');
  const bytes=new Uint8Array(22+png.length),view=new DataView(bytes.buffer);
  view.setUint16(2,1,true);view.setUint16(4,1,true);view.setUint8(6,width===256?0:width);view.setUint8(7,height===256?0:height);
  view.setUint16(10,1,true);view.setUint16(12,32,true);view.setUint32(14,png.length,true);view.setUint32(18,22,true);bytes.set(png,22);return bytes;
}
function outputName(original,format,used) {
  let base=String(original).replace(/\.[^/.]+$/,'').replace(/[\x00-\x1f\\/:*?"<>|]/g,'_').replace(/^[ .]+|[ .]+$/g,'').slice(0,120)||'图片';
  if(/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(base))base='图片_'+base;
  let name=base+'.'+format,index=1;while(used.has(name.toLowerCase()))name=base+' ('+(++index)+').'+format;return name;
}
export async function convertBatch(files,options,convert,progress=()=>{}) {
  validateOptions(options);if(files.length>32)throw new Error('一次最多转换32张图片');
  const used=new Set(),result=[];let size=0;
  for(let index=0;index<files.length;index++){
    const file=files[index],name=outputName(file.name,options.format,used);progress(index,'converting');
    try {
      const blob=await convert(file,options);
      if(!(blob instanceof Blob)||blob.size===0)throw new Error('没有生成有效图片');
      if(size+blob.size>32*1024*1024)throw new Error('转换结果超过32 MiB，请缩小尺寸或分批转换');
      size+=blob.size;used.add(name.toLowerCase());result.push({file,name,blob,error:''});progress(index,'success');
    } catch(error) {result.push({file,name,blob:null,error:String(error?.message||error)});progress(index,'error');}
  }
  return result;
}
export function outputMime(format) { if(!Object.hasOwn(formats,format))throw new Error('输出格式无效');return formats[format]; }
