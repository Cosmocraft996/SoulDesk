import test from 'node:test';
import assert from 'node:assert/strict';
const imported = await import('./scene-model.js').catch(() => null);
const scene = value => ({ type:'excalidraw', version:2, elements:[{ id:value, type:'rectangle' }], appState:{}, files:{} });

test('画板与图形库按实例恢复，损坏存储明确拒绝而不覆盖重置', () => {
  assert.ok(imported, '画板存储模型尚未实现');
  const { restoreScene } = imported;
  assert.equal(restoreScene({}).scene, null);
  const source = { scene:scene('a'), libraryItems:[{id:'library',elements:[]}], other:'retained' };
  assert.deepEqual(restoreScene(source), source);
  for (const value of [[],null,{scene:{}},{scene:{...scene('a'), files:{test:{dataURL:'https://external.example/img.png'}}}},{libraryItems:{}}]) assert.throws(() => restoreScene(value));
});

test('连续绘图串行保存最新图形与图形库，旧响应不能覆盖新草稿', async () => {
  assert.ok(imported);
  let finish;
  const first = new Promise(resolve => {finish=resolve;});
  const writes=[];
  const model=imported.createSceneStore({},async value=>{writes.push(value);if(writes.length===1)await first;});
  const saving=model.updateScene(scene('old'));
  model.updateScene(scene('new'));
  model.updateLibrary([{id:'shape',elements:[]}]);
  assert.equal(model.state.status,'saving');
  finish();await saving;
  assert.equal(writes.length,2);
  assert.equal(writes[1].scene.elements[0].id,'new');
  assert.equal(writes[1].libraryItems[0].id,'shape');
  assert.equal(model.state.status,'saved');
});

test('超限草稿不能被旧保存响应冒充已保存，修正后可以继续保存', async () => {
  assert.ok(imported);
  let finish;const first=new Promise(resolve=>{finish=resolve;});
  const model=imported.createSceneStore({},async()=>first,()=>{},2048);
  const saving=model.updateScene(scene('valid'));
  model.updateScene(scene('x'.repeat(3000)));
  assert.equal(model.state.status,'error');
  finish();await saving;
  assert.equal(model.state.status,'error');
  await model.updateScene(scene('repaired'));
  assert.equal(model.state.status,'saved');
});

test('磁盘保存失败保留最新画板和图形库，重试不丢内容', async () => {
  assert.ok(imported);let fails=true;const writes=[];
  const model=imported.createSceneStore({},async value=>{if(fails)throw new Error('保存失败');writes.push(value);});
  await model.updateScene(scene('draft'));
  assert.equal(model.state.status,'error');fails=false;
  await model.retry();
  assert.equal(writes[0].scene.elements[0].id,'draft');
  assert.equal(model.state.status,'saved');
});
