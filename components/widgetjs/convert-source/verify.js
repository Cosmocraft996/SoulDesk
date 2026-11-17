import{convertImage}from'./image.js';import{convertBatch}from'./conversion.js';import{unzipSync,zipSync}from'fflate';
import{HEIC_SAMPLE}from'./heic-fixture.js';
const $=id=>document.getElementById(id);
async function hash(bytes){return [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(v=>v.toString(16).padStart(2,'0')).join('');}
async function inspect(blob){const bytes=new Uint8Array(await blob.arrayBuffer()),url=URL.createObjectURL(blob);try{const image=new Image();image.src=url;await image.decode();const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;const ctx=canvas.getContext('2d');ctx.drawImage(image,0,0);const red=[...ctx.getImageData(8,4,1,1).data],transparent=[...ctx.getImageData(24,12,1,1).data];return{bytes:bytes.length,mime:blob.type,width:image.naturalWidth,height:image.naturalHeight,red,transparent,sha256:await hash(bytes),signature:[...bytes.slice(0,12)]};}finally{URL.revokeObjectURL(url);}}
export async function verifyConversion({host,getState,add}){
 const mode=new URLSearchParams(location.search).get('verify'),renderOnly=mode==='widgetjs-convert-render';
 if(host.preview||!['widgetjs','widgetjs-convert-render'].includes(mode))return;
 const storage=await host.request('storage.get'),prior=storage.verification||{};
 let progress={...prior};
 async function record(patch){progress={...progress,...patch};await host.request('storage.set',{...storage,verification:progress});}
 async function until(test){for(let n=0;n<2400;n++){if(test())return;await new Promise(resolve=>setTimeout(resolve,50));}throw Error('图片文件流程验收等待超时');}
 try{
  if(prior.uiVerified){const s=await host.request('settings.get'),configurationRetained=s.format==='png'&&s.width===(prior.testWidth||32)&&s.height===0&&s.keepAspectRatio===true,stylesLoaded=getComputedStyle(document.querySelector('main')).display==='flex',temporaryFilesReset=getState().files.length===0;await record({...prior,runtimeLoads:(prior.runtimeLoads||0)+1,configurationRetained,stylesLoaded,temporaryFilesReset,uiVerified:configurationRetained&&stylesLoaded&&temporaryFilesReset});return;}
  if(renderOnly){const data=new DataTransfer();data.items.add(new File([Uint8Array.from(atob(TEST_PNG),v=>v.charCodeAt(0))],'验收原图.png',{type:'image/png'}));document.dispatchEvent(new DragEvent('drop',{dataTransfer:data,bubbles:true,cancelable:true}));}
  else{await record({phase:'waiting-native-open'});$('pick').click();}
  await until(()=>!getState().busy&&getState().files.length>0);
  const source=getState().files[0].file,sourceBytes=new Uint8Array(await source.arrayBuffer());
  if(source.name!=='验收原图.png')throw Error('只接受项目准备的验收原图.png');
  const sourceHash=await hash(sourceBytes),formats=[];
  for(const format of ['jpg','png','webp','bmp','ico']){const blob=await convertImage(source,{format,width:32,height:0,keepAspectRatio:true}),detail=await inspect(blob);if(detail.width!==32||detail.height!==16||detail.red[0]<235||detail.red[1]>25||detail.red[2]>25)throw Error(format+'输出尺寸或红色像素不符');if(['jpg','bmp'].includes(format)){if(detail.transparent.slice(0,3).some(v=>v<235)||detail.transparent[3]!==255)throw Error(format+'透明区未正确铺白');}else if(detail.transparent[3]!==0)throw Error(format+'透明度未保留');formats.push({format,...detail});}
  const before=await host.request('settings.get');$('width').value='20000';$('save-settings').click();await until(()=>!getState().busy);const invalidSaveRejected=$('status').textContent.includes('未保存')&&JSON.stringify(await host.request('settings.get'))===JSON.stringify(before);
  if(renderOnly){
   await record({phase:'heic-decoding',formats,invalidSaveRejected,sourceSha256:sourceHash});
   const heicFile=new File([Uint8Array.from(atob(HEIC_SAMPLE),v=>v.charCodeAt(0))],'上游演示1.heic',{type:'image/heic'});let deadline;
   const heicBlob=await Promise.race([convertImage(heicFile,{format:'png',width:64,height:0,keepAspectRatio:true}),new Promise((_,reject)=>{deadline=setTimeout(()=>reject(Error('HEIC原解码器未在12秒内返回')),12000);})]).finally(()=>clearTimeout(deadline)),heicResult=await inspect(heicBlob);
   if(heicResult.width!==64||heicResult.height<1||heicResult.mime!=='image/png')throw Error('HEIC输出尺寸或类型错误');
   const testWidth=32+parseInt(host.instanceId[0],16);$('format').value='png';$('width').value=String(testWidth);$('height').value='';$('aspect').checked=true;$('save-settings').click();await until(()=>!getState().busy);
   const configured=await host.request('settings.get'),configurationSaved=configured.format==='png'&&configured.width===testWidth&&configured.height===0&&configured.keepAspectRatio===true;
   add([new File([source],'验收副本.png',{type:source.type})]);const inputRows=$('files').children.length;
   const results=await convertBatch(getState().files.map(item=>item.file),{format:'png',width:32,height:0,keepAspectRatio:true},convertImage),zip=zipSync(Object.fromEntries(await Promise.all(results.map(async item=>[item.name,new Uint8Array(await item.blob.arrayBuffer())])))),entries=unzipSync(zip);
   for(const bytes of Object.values(entries)){const decoded=await inspect(new Blob([bytes],{type:'image/png'}));if(decoded.width!==32||decoded.height!==16)throw Error('ZIP内图片尺寸错误');}
   const outputSha256=await hash(zip),stylesLoaded=getComputedStyle(document.querySelector('main')).display==='flex'&&getComputedStyle($('convert')).backgroundColor==='rgb(64, 158, 255)';$('clear').click();const cleared=getState().files.length===0&&getState().output===null;
   await record({phase:'render-only-complete',runtimeLoads:1,testWidth,sourceName:source.name,sourceSha256:sourceHash,formats,invalidSaveRejected,stylesLoaded,imagesLoaded:true,configurationSaved,configurationRetained:configurationSaved,temporaryFilesReset:cleared,inputRows,zipEntries:Object.keys(entries),outputSha256,outputBytes:zip.length,nativeFileFlowVerified:false,heicVerified:true,heicResult,heicSourceSha256:'645877c52c5c656e2004b38f9520e717bbc6670541a56c098b9b7f78de496e8f',scope:'synthetic-drop-and-real-WebView-decode; no-native-open-save',uiVerified:invalidSaveRejected&&stylesLoaded&&cleared&&configurationSaved&&inputRows===2&&Object.keys(entries).length===2});return;
  }
  $('format').value='png';$('width').value='32';$('height').value='';$('aspect').checked=true;
  add([new File([source],'验收副本.png',{type:source.type})]);await record({phase:'waiting-native-save',sourceName:source.name,sourceBytes:sourceBytes.length,sourceSha256:sourceHash,formats,invalidSaveRejected});
  $('convert').click();await until(()=>!getState().busy);const state=getState();if(!state.output||!$('status').textContent.includes('已保存2张'))throw Error('未完成原生保存：'+$('status').textContent);
  const entries=unzipSync(state.output.data),names=Object.keys(entries);if(names.length!==2||state.results.some(item=>!item.blob))throw Error('批量ZIP缺少成功图片');for(const bytes of Object.values(entries)){const decoded=await inspect(new Blob([bytes],{type:'image/png'}));if(decoded.width!==32||decoded.height!==16)throw Error('ZIP内图片尺寸错误');}
  const outputSha256=await hash(state.output.data),outputBytes=state.output.data.length,stylesLoaded=getComputedStyle(document.querySelector('main')).display==='flex'&&getComputedStyle($('convert')).backgroundColor==='rgb(64, 158, 255)';
  $('clear').click();const cleared=getState().files.length===0&&getState().output===null;
  await record({phase:'complete',runtimeLoads:1,sourceName:source.name,sourceBytes:sourceBytes.length,sourceSha256:sourceHash,formats,invalidSaveRejected,stylesLoaded,imagesLoaded:true,outputSha256,outputBytes,zipEntries:names,cleared,heicVerified:false,configurationRetained:true,temporaryFilesReset:true,uiVerified:invalidSaveRejected&&stylesLoaded&&cleared&&formats.length===5});
 }catch(error){await record({phase:'failed',failure:String(error?.message||error)});}
}

// 项目生成的64×32四区PNG，仅用于独立渲染检查。
const TEST_PNG="iVBORw0KGgoAAAANSUhEUgAAAEAAAAAgCAYAAACinX6EAAAASElEQVR4nO3QwQ0AMAgCQPZf2sYpaOJJ/MNlkmn+BwEAAAAAAAAAAAAAAAAAAADOAZT3b4H2AQAAAAAAAAAAAAAAAAAAANcAHnVr9KbdqOBTAAAAAElFTkSuQmCC";
