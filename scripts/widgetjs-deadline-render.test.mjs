import test from'node:test';import assert from'node:assert/strict';import{readFileSync}from'node:fs';import{createHash}from'node:crypto';import{createSSRApp}from'vue';import{renderToString}from'vue/server-renderer';import{createServer}from'vite';import{resolve}from'node:path';import{extractDeadlineSvg}from'./port-widgetjs-deadline.mjs';
test('Deadline原SVG和动画从固定部署提取，来源散列完整，不执行外部表达式',()=>{
  const root='components/widgetjs/deadline-source/',source=readFileSync(root+'original/DeadlineWidgetView-IuZHNBhI.js','utf8'),svg=readFileSync(root+'artwork.svg','utf8'),provenance=JSON.parse(readFileSync(root+'SOURCE_FILES.json','utf8'));
  assert.equal(svg,extractDeadlineSvg(source));assert.equal(provenance.files.length,5);for(const file of provenance.files)assert.equal(createHash('sha256').update(readFileSync(root+file.path)).digest('hex'),file.sha256,file.path);
  for(const id of ['red-flame','yellow-flame','white-flame','progress-time-fill','death-group','death-arm','death-tool','designer-body','designer-arm-grop','designer-pen'])assert.ok(svg.includes('id="'+id+'"'),id);
  assert.ok((svg.match(/<path /g)||[]).length>=12);assert.ok(!/https?:|<script|onload=/.test(svg));
  assert.throws(()=>extractDeadlineSvg('e("svg",{},[e("path",{d:alert(1)})])'),/非静态/);
  assert.throws(()=>extractDeadlineSvg('e("svg",{},[e("use",{"xlink:href":"https://evil.test"})])'),/外部引用/);
  const css=readFileSync(root+'original-style.css','utf8');assert.equal(css,readFileSync(root+'original/DeadlineWidgetView-C9hVe0nS.css','utf8').replaceAll('[data-v-afe3cbb0]',''));assert.match(css,/show-flames-afe3cbb0 20s/);assert.match(css,/write-afe3cbb0 1.5s/);
});
test('Deadline实际Vue模板渲染原图和文本，设置成功后应用、失败保留草稿及旧配置',async()=>{
  const oldCSS=globalThis.CSS;globalThis.CSS={supports:()=>true};const vite=await createServer({configFile:resolve('components/widgetjs/deadline-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,watch:null},appType:'custom',logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]}});
  try{const{default:App}=await vite.ssrLoadModule('/App.vue');let state,fail=false;const writes=[],initialSettings={title:'截止 <测试>',startTime:'2026-11-14T00:00:00.000Z',endTime:'2026-11-16T00:00:00.000Z',primaryColor:'#BE002A',background:'rgba(0,0,0,0.5)',radius:22};const host={preview:false,async request(method,args){assert.equal(method,'settings.update');if(fail)throw Error('写入失败');writes.push(structuredClone(args));return args;}};
    const Subject={...App,setup(props,context){state=App.setup(props,context);return state;}};const html=await renderToString(createSSRApp(Subject,{host,initialSettings}));assert.match(html,/id="designer-arm-grop"/);assert.match(html,/截止 &lt;测试&gt; 剩余/);assert.match(html,/type="date"/);assert.ok(!html.includes('NaN'));
    let closed=0;state.dialog.value={showModal(){},close(){closed++;}};state.open();state.draft.value.title='已保存';await state.save();assert.equal(writes.length,1);assert.equal(state.settings.value.title,'已保存');assert.equal(closed,1);
    state.open();state.draft.value.title='失败草稿';fail=true;await state.save();assert.equal(state.settings.value.title,'已保存');assert.equal(state.draft.value.title,'失败草稿');assert.match(state.saveError.value,/设置未保存.*写入失败/);assert.equal(closed,1);
    fail=false;state.draft.value.endDate=state.draft.value.startDate;await state.save();assert.equal(writes.length,1);assert.match(state.saveError.value,/至少一分钟/);
  }finally{await vite.close();if(oldCSS===undefined)delete globalThis.CSS;else globalThis.CSS=oldCSS;}
});
