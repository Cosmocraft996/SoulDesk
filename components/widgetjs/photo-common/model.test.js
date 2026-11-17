import test from 'node:test';
import assert from 'node:assert/strict';
const module = await import('./model.js').catch(()=>null);
const listing = {name:'相册',revision:'r1',files:[{name:'a.jpg'},{name:'b.png'},{name:'c.webp'}]};

test('相册支持前后循环、随机顺序及空目录，不修改宿主列表',async()=>{
  assert.ok(module,'相册模型尚未实现');
  const model = module.createCarousel(async(name)=>name);
  await model.load(listing,false);
  assert.equal(model.state.name,'a.jpg');
  await model.move(-1);assert.equal(model.state.name,'c.webp');
  await model.move(1);assert.equal(model.state.name,'a.jpg');
  await model.load(listing,true,()=>0);
  assert.deepEqual(model.state.files,['b.png','c.webp','a.jpg']);
  assert.equal(listing.files[0].name,'a.jpg');
  await model.load({...listing,files:[]},false);
  assert.equal(model.state.image,null);assert.equal(model.state.name,'');
  await model.move(1);assert.equal(model.state.error,'');
});

test('旧目录与旧切换响应不能覆盖新图，读取失败保留当前照片并可重试',async()=>{
  assert.ok(module);
  const pending=[];
  const model=module.createCarousel((name,revision)=>new Promise((resolve,reject)=>pending.push({name,revision,resolve,reject})));
  const first=model.load(listing,false);pending[0].resolve('image-a');await first;
  const old=model.move(1);
  const current=model.load({...listing,revision:'r2',files:[{name:'new.png'}]},false);
  pending[2].resolve('image-new');await current;
  pending[1].resolve('image-old');await old;
  assert.equal(model.state.image,'image-new');
  const failed=model.move(1);pending[3].reject(new Error('图片不可读'));await failed;
  assert.equal(model.state.image,'image-new');assert.equal(model.state.error,'图片不可读');
  const retry=model.move(1);pending[4].resolve('image-retry');await retry;
  assert.equal(model.state.image,'image-retry');assert.equal(model.state.error,'');
  model.dispose();
});

test('大头贴尺寸不放大原图且不会产生负数，图片地址只允许HTTPS或受限图片数据',()=>{
  assert.ok(module);
  assert.deepEqual(module.stickerSize(1200,800,300,400),[236,157]);
  assert.deepEqual(module.stickerSize(50,30,300,400),[50,30]);
  assert.ok(module.stickerSize(400,200,30,30).every(value=>value>=1));
  for(const url of ['https://images.example.com/a.png','data:image/png;base64,YQ=='])assert.equal(module.validatePicture(url),url);
  for(const url of ['javascript:alert(1)','file:///C:/a.png','data:text/html,abc','http://example.com/a.png'])assert.throws(()=>module.validatePicture(url));
});
