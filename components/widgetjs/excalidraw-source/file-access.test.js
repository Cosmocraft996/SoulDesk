import test from 'node:test';
import assert from 'node:assert/strict';
const imported=await import('./file-access.js').catch(()=>null);

test('文件打开使用用户选择接口，保留图片字节，取消与预览不修改画板', async()=>{
  assert.ok(imported,'文件适配尚未实现');const requests=[];
  const adapter=imported.createFileAccess({preview:false,request:async(method,args)=>{requests.push({method,args});return [{name:'图.png',mime:'image/png',data:'AQID'}];}});
  const file=await adapter.fileOpen({extensions:['.png'],multiple:false});
  assert.equal(file.name,'图.png');assert.deepEqual([...new Uint8Array(await file.arrayBuffer())],[1,2,3]);
  assert.equal(requests[0].method,'documents.open');assert.deepEqual(requests[0].args.extensions,['png']);
  await assert.rejects(imported.createFileAccess({request:async()=>null}).fileOpen(),{name:'AbortError'});
  let previewCalls=0;
  await assert.rejects(imported.createFileAccess({preview:true,request:()=>{previewCalls++;}}).fileOpen(),{name:'AbortError'});
  assert.equal(previewCalls,0);
});

test('导出必须等待原生保存成功，失败与取消不返回成功结果',async()=>{
  assert.ok(imported);let resolve;const pending=new Promise(r=>{resolve=r;});let saved=false;
  const adapter=imported.createFileAccess({request:()=>pending});
  const saving=adapter.fileSave(new Blob(['scene']),{fileName:'画板',extensions:['.excalidraw']}).then(()=>{saved=true;});
  await new Promise(r=>setImmediate(r));assert.equal(saved,false);
  resolve({name:'画板.excalidraw'});await saving;assert.equal(saved,true);
  await assert.rejects(imported.createFileAccess({request:async()=>{throw new Error('磁盘只读');}}).fileSave(new Blob(['scene']),{fileName:'x.json',extensions:['.json']}),/磁盘只读/);
  await assert.rejects(imported.createFileAccess({request:async()=>null}).fileSave(new Blob(['scene']),{fileName:'x.json',extensions:['.json']}),{name:'AbortError'});
});
