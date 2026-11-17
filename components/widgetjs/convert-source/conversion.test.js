import test from 'node:test';
import assert from 'node:assert/strict';
const module=await import('./conversion.js').catch(()=>null);
test('BMP输出具有正确行对齐、BGR像素和从上到下的行序',()=>{
  assert.ok(module,'图片转换算法尚未实现');
  const data=module.encodeBmp(2,2,new Uint8ClampedArray([255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255]));
  const view=new DataView(data.buffer);
  assert.equal(new TextDecoder().decode(data.subarray(0,2)),'BM');
  assert.equal(view.getUint32(2,true),70);assert.equal(view.getInt32(22,true),-2);
  assert.deepEqual([...data.subarray(54,62)],[0,0,255,0,255,0,0,0]);
  assert.deepEqual([...data.subarray(62,70)],[255,0,0,255,255,255,0,0]);
});
test('ICO封装保留PNG字节，256尺寸使用零编码，拒绝错误的超大尺寸',()=>{
  assert.ok(module);
  const png=Uint8Array.from(Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a7toAAAAASUVORK5CYII=','base64'));
  const data=module.encodeIco(256,128,png),view=new DataView(data.buffer);
  assert.equal(view.getUint16(2,true),1);assert.equal(view.getUint16(4,true),1);
  assert.equal(data[6],0);assert.equal(data[7],128);assert.equal(view.getUint32(18,true),22);
  assert.deepEqual(data.subarray(22),png);assert.throws(()=>module.encodeIco(300,128,png));
});
test('尺寸保持原版单边比例及双边指定行为，ICO自动缩小且画布有容量边界',()=>{
  assert.ok(module);
  const base={format:'png',width:0,height:0,keepAspectRatio:true};
  assert.deepEqual(module.outputDimensions(600,400,{...base,width:300}),[300,200]);
  assert.deepEqual(module.outputDimensions(600,400,{...base,width:300,keepAspectRatio:false}),[300,400]);
  assert.deepEqual(module.outputDimensions(600,400,{...base,width:300,height:100}),[300,100]);
  assert.deepEqual(module.outputDimensions(1200,800,{...base,format:'ico'}),[256,171]);
  assert.throws(()=>module.outputDimensions(600,400,{...base,format:'ico',width:300}));
  assert.throws(()=>module.outputDimensions(16000,16000,base));
});
test('批量转换保留逐项失败与成功数据，同名输出不会互相覆盖',async()=>{
  assert.ok(module);
  const files=[{name:'a.jpg'},{name:'a.png'},{name:'坏图.heic'}],progress=[];
  const result=await module.convertBatch(files,{format:'webp',width:0,height:0,keepAspectRatio:true},async file=>{if(file.name.endsWith('heic'))throw new Error('HEIC解码失败');return new Blob([file.name],{type:'image/webp'});},(index,state)=>progress.push([index,state]));
  assert.deepEqual(result.map(item=>item.name),['a.webp','a (2).webp','坏图.webp']);
  assert.equal(await result[0].blob.text(),'a.jpg');assert.equal(result[2].error,'HEIC解码失败');
  assert.deepEqual(progress.at(-1),[2,'error']);
});
