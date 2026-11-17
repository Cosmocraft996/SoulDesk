import test from 'node:test';
import assert from 'node:assert/strict';
import {restoreDocuments,readImport,createDocumentStore} from './document-model.js';
import {graphShortcutAllowed} from './keyboard-guard.js';
import {createPng} from './png-export.js';
import Hierarchy from '@antv/hierarchy';
import {transformData,getMindNodeData,getVisibleData} from './editor/mind-map/utils.ts';
const node=(name='主题')=>({id:'root',name,checked:false,collapsed:false,children:[{id:'child',name:'分支',url:'https://example.com',checked:true,collapsed:true,children:[]}]});
const fixture=()=>({version:1,documents:[{id:'one',topic:'主题',group:'默认分组',emoji:'📝',createdAt:1,updatedAt:2,mindmap:node()}],preferences:{theme:'system',showGrid:true,showCheckboxes:true,sidebarOpen:true}});
test('还原保留原树全部字段和实例偏好，损坏数据拒绝重置覆盖',()=>{
  const input=fixture(),output=restoreDocuments(input);assert.deepEqual(output,input);input.documents[0].mindmap.children[0].name='外部修改';assert.equal(output.documents[0].mindmap.children[0].name,'分支');
  assert.equal(restoreDocuments({}).documents.length,0);assert.throws(()=>restoreDocuments({version:2,documents:[]}),/格式|版本/);const duplicate=fixture();duplicate.documents[0].mindmap.children[0].id='root';assert.throws(()=>restoreDocuments(duplicate),/标识/);
});
test('连续编辑串行保存最新文档，早期响应不把新内容标为已保存',async()=>{
  const writes=[];const model=createDocumentStore(fixture(),value=>new Promise(resolve=>writes.push({value,resolve})));
  model.updateTree('one',node('第一版'));assert.equal(writes.length,1);model.updateTree('one',node('第二版'));assert.equal(writes.length,1);assert.equal(model.state.status,'saving');writes[0].resolve();await Promise.resolve();await Promise.resolve();assert.equal(writes.length,2);assert.equal(writes[1].value.documents[0].mindmap.name,'第二版');assert.equal(model.state.status,'saving');writes[1].resolve();assert.equal(await model.flush(),true);assert.equal(model.state.status,'saved');
});
test('保存失败保留可导出草稿，重试成功才显示已保存',async()=>{
  let fail=true,saved;const model=createDocumentStore(fixture(),async value=>{if(fail)throw new Error('磁盘写入失败');saved=value;});model.updateTree('one',node('未保存草稿'));assert.equal(await model.flush(),false);assert.equal(model.state.status,'error');assert.match(model.state.error,/磁盘/);assert.equal(model.snapshot().documents[0].mindmap.name,'未保存草稿');fail=false;assert.equal(await model.retry(),true);assert.equal(saved.documents[0].mindmap.name,'未保存草稿');assert.equal(model.state.status,'saved');
});
test('创建分组、改名、删除与偏好变更按实例独立保存',async()=>{
  let sequence=0;const model=createDocumentStore({},async()=>{},()=>{},32*1024*1024,()=> 'new-'+(++sequence),()=>1000);
  const id=model.create('工作计划','📚','项目');assert.equal(model.snapshot().documents[0].id,id);model.rename(id,'新主题','💡','新分组');assert.equal(model.snapshot().documents[0].mindmap.name,'新主题');assert.equal(model.snapshot().documents[0].group,'新分组');model.setPreferences({theme:'dark',showGrid:false,sidebarOpen:false});await model.flush();assert.equal(model.snapshot().preferences.theme,'dark');assert.equal(restoreDocuments({}).documents.length,0);model.remove(id);assert.equal(model.snapshot().documents.length,0);
});
test('导入只新增副本，坏树、循环、重复标识和深层数据不覆盖已有文档',async()=>{
  const model=createDocumentStore(fixture(),async()=>{},()=>{},32*1024*1024,()=>crypto.randomUUID(),()=>1000);const imported=readImport(JSON.stringify({topic:'导入主题',emoji:'📖',group:'导入',mindmap:node()}));model.importDocument(imported);await model.flush();assert.equal(model.snapshot().documents.length,2);assert.equal(model.snapshot().documents[0].id,'one');assert.notEqual(model.snapshot().documents[1].id,'one');assert.equal(model.snapshot().documents[1].topic,'导入主题');
  assert.equal(readImport(JSON.stringify(node())).mindmap.name,'主题');assert.throws(()=>readImport('{broken'),/JSON|格式/);const bad=node();bad.children=[bad];assert.throws(()=>model.updateTree('one',bad),/循环|树|标识/);const deep=node();let cursor=deep;for(let i=0;i<140;i++){cursor.children=[{id:'depth-'+i,name:'深层'}];cursor=cursor.children[0];}assert.throws(()=>model.updateTree('one',deep),/层级|深度/);assert.equal(model.snapshot().documents.length,2);
});
test('超出存储上限不发送，保留最新草稿并允许缩小后恢复保存',async()=>{
  let writes=0;const model=createDocumentStore(fixture(),async()=>{writes++;},()=>{},1800);model.updateTree('one',node('字'.repeat(1000)));assert.equal(await model.flush(),false);assert.equal(writes,0);assert.equal(model.snapshot().documents[0].mindmap.name.length,1000);assert.match(model.state.error,/上限/);model.updateTree('one',node('已缩小'));assert.equal(await model.flush(),true);assert.equal(writes,1);assert.equal(model.state.status,'saved');
});
test('离开组件后拒绝新变更，不以迟到响应更新界面状态',async()=>{
  let finish,notices=0;const model=createDocumentStore(fixture(),()=>new Promise(resolve=>finish=resolve),()=>notices++);model.updateTree('one',node('编辑中'));const count=notices;model.dispose();finish();await model.flush();assert.equal(notices,count);assert.throws(()=>model.remove('one'),/关闭|离开/);
});
test('图编辑快捷键限于当前画布，不抢设置、弹窗或侧栏的按键',()=>{
  const event=(tagName,canvas=true,dialog=false)=>({key:'Tab',target:{tagName,isContentEditable:false,closest:selector=>selector==='[data-mindmap-canvas]'?canvas:dialog},defaultPrevented:false});
  assert.equal(graphShortcutAllowed(event('DIV')),true);assert.equal(graphShortcutAllowed(event('INPUT')),false);assert.equal(graphShortcutAllowed(event('BUTTON')),false);assert.equal(graphShortcutAllowed(event('DIV',false)),false);assert.equal(graphShortcutAllowed(event('DIV',true,true)),false);assert.equal(graphShortcutAllowed({...event('DIV'),ctrlKey:true}),false);
});
test('PNG生成失败或未回调时明确报错，超时后的迟到图像不触发保存',async()=>{
  await assert.rejects(createPng({toPNG:callback=>callback('not-png')},10),/PNG/);await assert.rejects(createPng({toPNG:()=>{throw new Error('渲染失败');}},10),/渲染/);
  let late;await assert.rejects(createPng({toPNG:callback=>late=callback},5),/超时/);late('data:image/png;base64,aGVsbG8=');
  assert.equal(await createPng({toPNG:callback=>callback('data:image/png;base64,aGVsbG8=')},10),'data:image/png;base64,aGVsbG8=');
});
test('原编辑器树转换保留URL、勾选和折叠，收起不丢失深层子节点',()=>{
  const tree=node();tree.children[0].children=[{id:'deep',name:'仍需保存',checked:true,collapsed:false,children:[]}];const transformed=transformData(tree),visible=getVisibleData(transformed);assert.equal(visible.children[0].children.length,0);const restored=getMindNodeData(transformed);assert.equal(restored.children[0].children[0].name,'仍需保存');assert.equal(restored.children[0].url,'https://example.com');assert.equal(restored.children[0].checked,true);assert.equal(restored.children[0].collapsed,true);
});
test('原AntV层次布局实际执行，折叠与展开得到有限节点坐标',()=>{
  const tree=node();tree.children[0].children=[{id:'deep',name:'深层分支'}];
  const layout=value=>Hierarchy.mindmap(getVisibleData(transformData(value)),{direction:'H',getHeight:()=>36,getWidth:()=>180,getHGap:()=>60,getVGap:()=>10,getSide:()=> 'right'});
  const first=layout(tree);assert.equal(first.children[0].children?.length||0,0);tree.children[0].collapsed=false;const second=layout(tree);assert.equal(second.children[0].children[0].id,'deep');for(const item of [second,second.children[0],second.children[0].children[0]]){assert.ok(Number.isFinite(item.x));assert.ok(Number.isFinite(item.y));}
});
