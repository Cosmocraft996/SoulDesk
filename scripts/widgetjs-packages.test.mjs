import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { posix } from 'node:path';
import ts from 'typescript';
import { widgetFiles, portedWidgets } from './build-widgetjs.mjs';
import { coins, fallbacks } from '../components/widgetjs/coin-medium-source/coin-model.js';

function imports(path, bytes) {
  const source = ts.createSourceFile(path, new TextDecoder().decode(bytes), ts.ScriptTarget.Latest, true, ts.ScriptKind.JS), references = [];
  function visit(node) {
    const specifier = ts.isImportDeclaration(node) || ts.isExportDeclaration(node) ? node.moduleSpecifier
      : ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword ? node.arguments[0] : undefined;
    if (specifier && ts.isStringLiteralLike(specifier)) references.push({ path: specifier.text, dynamic: ts.isCallExpression(node) });
    ts.forEachChild(node, visit);
  }
  visit(source); return references;
}

test('六款角色包的静态/动态模块全部自含，普通入口不加载图形引擎', () => {
  for (const name of ['ai-assistant', 'ai-mindmap', 'ai-mermaid', 'ai-translate', 'clipboard', 'clipboard-search']) {
    const files = widgetFiles(name), graph = new Map();
    for (const [path, bytes] of Object.entries(files).filter(([path]) => path.endsWith('.js'))) {
      const references = imports(path, bytes);
      for (const ref of references) {
        assert.ok(ref.path.startsWith('.'), `${name}/${path}存在包外模块：${ref.path}`);
        ref.target = posix.normalize(posix.join(posix.dirname(path), ref.path));
        assert.ok(files[ref.target], `${name}/${path}缺少模块：${ref.target}`);
      }
      graph.set(path, references);
    }
    const initial = new Set();
    function visit(path) { if (initial.has(path)) return; initial.add(path); for (const ref of graph.get(path) ?? []) if (!ref.dynamic) visit(ref.target); }
    visit('dist/app.js');
    assert.ok([...initial].reduce((n, path) => n + files[path].length, 0) < 1300000, `${name}入口重新带入了重型依赖`);
    const licenses = new TextDecoder().decode(files['dist/THIRD_PARTY_LICENSES.txt']);
    assert.equal(/^mermaid@/m.test(licenses), name === 'ai-mermaid', `${name}图表引擎归属错误`);
    assert.equal(/^markmap-lib@/m.test(licenses), ['ai-mindmap', 'clipboard-search'].includes(name), `${name}导图引擎归属错误`);
    for (const [path, bytes] of Object.entries(files).filter(([path]) => path.endsWith('.css'))) {
      for (const match of new TextDecoder().decode(bytes).matchAll(/url\(\s*["']?(\.[^\s"')]+)["']?\s*\)/g)) {
        assert.ok(files[posix.normalize(posix.join(posix.dirname(path), match[1]))], `${name}缺少样式资源：${match[1]}`);
      }
    }
  }
});
test('AI直聘标准包使用固定能力并保留部署来源、原版权声明和全部UI资源',()=>{const files=widgetFiles('ai-job'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));assert.deepEqual(manifest.capabilities,['jobs.manage','clipboard.write']);assert.equal(manifest.id,'local.souldesk.widgetjs.ai-job');assert.ok(files[manifest.entry]);assert.match(new TextDecoder().decode(files['dist/SOURCE.txt']),/58806a7f80ab5389b49087dd66498bdd7c1982c2578cb199ead31b4e27d3d11f/);assert.match(new TextDecoder().decode(files['dist/UPSTREAM_NOTICES.txt']),/@license MIT/);assert.ok(files['dist/THIRD_PARTY_LICENSES.txt']);assert.ok(files['dist/app.js'].length>1000000);assert.ok(Object.keys(files).filter(path=>path.endsWith('.png')).length>=10);});

test('网页助手包只声明固定能力，保留八站图标与原来源，预览不使用远程iframe',()=>{
  const files=widgetFiles('ai-assistant'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
  assert.deepEqual(manifest.capabilities,['assistant.web']);assert.equal(manifest.id,'local.souldesk.widgetjs.ai-assistant');
  assert.match(new TextDecoder().decode(files[manifest.entry]),/data-ai-variant="assistant"/);
  assert.match(new TextDecoder().decode(files['dist/SOURCE.txt']),/0160f55feabd19421953c687eba033a707a8c3c2/);
  const source=JSON.parse(readFileSync('components/widgetjs/ai-source/SOURCE_FILES.json','utf8'));
  const names=['robot.png','chatgpt-icon.png','grok.png','google-gemini-icon.png','deepseek.png','kimi.png','qwen.png','dou_bao.png','yuanbao.png'];
  for(const name of names){const original=source.files.find(f=>posix.basename(f.path)===name);assert.ok(original,name);assert.equal(createHash('sha256').update(files['dist/'+name]).digest('hex'),original.sha256,name);}
  assert.match(new TextDecoder().decode(files['dist/LICENSE.txt']),/GNU GENERAL PUBLIC LICENSE/);assert.ok(files['dist/THIRD_PARTY_LICENSES.txt']);
});

test('两款剪切板包按类型声明固定能力，保留51份来源且入口无密钥或远程AI页',()=>{
  const source=JSON.parse(readFileSync('components/widgetjs/clipboard-source/SOURCE_FILES.json','utf8'));assert.equal(source.revision,'2482fc6b238983f3fe8f2aecf042d92a9b29c9e2');assert.equal(source.files.length,51);
  for(const file of source.files)assert.equal(createHash('sha256').update(readFileSync('components/widgetjs/clipboard-source/'+file.path)).digest('hex'),file.sha256,file.path);
  for(const[name,kind,caps]of[['clipboard','history',['clipboard.control','clipboard.history','clipboard.copy','clipboard.paste']],['clipboard-search','search',['clipboard.control','clipboard.search','clipboard.tools','ai.generate','documents.save','documents.drop','clipboard.write']]]){
    const files=widgetFiles(name),m=JSON.parse(new TextDecoder().decode(files['manifest.json']));assert.deepEqual(m.capabilities,caps);const html=new TextDecoder().decode(files[m.entry]);assert.match(html,new RegExp('data-clipboard-kind="'+kind+'"'));assert.doesNotMatch(html,/apiKey|widgetjs.cn\/ai\/page/);assert.match(new TextDecoder().decode(files['dist/SOURCE.txt']),/2482fc6b/);assert.match(new TextDecoder().decode(files['dist/LICENSE-AI.txt']),/GNU GENERAL PUBLIC LICENSE/);assert.ok(files['dist/THIRD_PARTY_LICENSES.txt']);
  }
});

test('三款AI生成包各带所用原图标与固定能力，入口按类型隔离且不携带密钥',()=>{
  const source=JSON.parse(readFileSync('components/widgetjs/ai-source/SOURCE_FILES.json','utf8'));assert.equal(source.revision,'0160f55feabd19421953c687eba033a707a8c3c2');
  for(const name of ['ai-mindmap','ai-mermaid','ai-translate']){const files=widgetFiles(name),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));assert.ok(manifest.capabilities.includes('ai.generate'));assert.ok(manifest.capabilities.includes('storage.instance'));assert.ok(manifest.capabilities.includes('clipboard.write'));assert.equal(manifest.capabilities.includes('documents.save'),name!=='ai-translate');for(const cap of ['web.embed','assets.local','content.read','clipboard.read'])assert.ok(!manifest.capabilities.includes(cap));
    for(const file of source.files.filter(item=>posix.basename(item.path)===({'ai-mindmap':'exploding_head_3d.png','ai-mermaid':'bar_chart.png','ai-translate':'globe_showing_asia-australia_3d.png'}[name])))assert.equal(createHash('sha256').update(files['dist/'+posix.basename(file.path)]).digest('hex'),file.sha256);
    assert.match(new TextDecoder().decode(files[manifest.entry]),new RegExp('data-ai-variant="'+name.slice(3)+'"'));assert.match(new TextDecoder().decode(files['dist/LICENSE.txt']),/GNU GENERAL PUBLIC LICENSE/);assert.ok(files['dist/app.js'].length>100000,'组件仍携带真实Vue界面');
  }
});

test('三款提醒标准包包含六份原音频和四张原头像，预览不获得普通存储或通用窗口能力',()=>{
  const source=JSON.parse(readFileSync('components/widgetjs/reminder-source/SOURCE_FILES.json','utf8'));
  assert.equal(source.revision,'714802e66336133396ab55463eb5a26d08fc0788');
  for(const[name,cap]of[['dynamic-island','notifications.island'],['sit-reminder','reminders.sit'],['phone-reminder','reminders.phone']]){
    const files=widgetFiles(name),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));assert.deepEqual(manifest.capabilities,[cap]);
    assert.match(new TextDecoder().decode(files[manifest.entry]),new RegExp('data-reminder-variant="'+name+'"'));
    const assets=source.files.filter(file=>file.path.startsWith('original/public/'));assert.equal(assets.length,10);
    for(const item of assets){const bytes=files['dist/'+posix.basename(item.path)];assert.ok(bytes,item.path);assert.equal(createHash('sha256').update(bytes).digest('hex'),item.sha256);}
    assert.match(new TextDecoder().decode(files['dist/LICENSE.txt']),/GNU GENERAL PUBLIC LICENSE/);
  }
});

test('全部移植包的相对模块和HTML资源完整，原生预置包含每个移植项', () => {
  const ids = new Set(), presetSource = readFileSync('src-tauri/src/presets.rs', 'utf8');
  for (const name of portedWidgets) {
    const files = widgetFiles(name), decode = path => new TextDecoder().decode(files[path]);
    const manifest = JSON.parse(decode('manifest.json'));
    assert.ok(!ids.has(manifest.id)); ids.add(manifest.id);
    assert.ok(files[manifest.entry]);
    assert.ok(presetSource.includes('"' + name + '"') || presetSource.includes('components/widgetjs/' + name + '/manifest.json'), name + '未预置');
    for (const [path] of Object.entries(files)) {
      if (!/\.(js|html)$/.test(path)) continue;
      const relative = /(?:src|href)="(\.{1,2}\/[^\"]+)"|(?:from |import\()["'](\.{1,2}\/[^"']+)["']/g;
      for (const match of decode(path).matchAll(relative)) assert.ok(files[posix.normalize(posix.join(posix.dirname(path), match[1] || match[2]))], path + '缺少' + match[0]);
    }
  }
});

test('电子木鱼随包交付原音频和图片，不依赖远程地址', () => {
  const files = widgetFiles('wooden-fish');
  for (const target of ['wooden_fish.png', 'hit.mp3', 'bgm.mp3']) {
    assert.ok(files['dist/' + target]?.length > 1000, target);
  }
});

test('倒计时列表独立包选择列表视图，入口不依赖宿主CSP禁止的内联脚本',()=>{
  const files=widgetFiles('countdown-list'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json'])),html=new TextDecoder().decode(files[manifest.entry]);
  assert.match(html,/<body\s[^>]*data-countdown-variant="countdown-list"/);
  for(const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g))assert.ok(/\bsrc="/.test(match[1])||!match[2].trim(),'组件入口包含会被CSP拒绝的内联脚本');
});

test('待办本地包保留原完成音效，声明核心待办能力并保留固定部署来源',()=>{
  const files=widgetFiles('todo'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json'])),provenance=JSON.parse(readFileSync('components/widgetjs/todo-source/SOURCE_FILES.json','utf8'));
  assert.ok(files['dist/ding-B9BOXEwg.mp3']?.length>15000);assert.ok(manifest.capabilities.includes('todos.manage'));assert.ok(manifest.capabilities.includes('storage.instance'));assert.equal(provenance.upstreamId,'itime.fun.todo_list');
  const audio=provenance.files.find(item=>item.path==='original/ding-B9BOXEwg.mp3');assert.ok(audio);assert.equal(createHash('sha256').update(files['dist/ding-B9BOXEwg.mp3']).digest('hex'),audio.sha256);
  for(const item of provenance.files)assert.equal(createHash('sha256').update(readFileSync('components/widgetjs/todo-source/'+item.path)).digest('hex'),item.sha256,item.path);
});
test('大号日历标准包携带原引擎样式，声明固定公开源与待办能力，设置仅含原四项',()=>{const files=widgetFiles('calendar-large'),m=JSON.parse(new TextDecoder().decode(files['manifest.json'])),css=new TextDecoder().decode(files['dist/style.css']);for(const cap of ['storage.instance','todos.manage','content.source.itime-calendar'])assert.ok(m.capabilities.includes(cap));assert.ok(!m.capabilities.includes('content.read'));assert.deepEqual(Object.keys(m.settingsSchema.properties).sort(),['background','firstDayOfWeek','foreground','radius']);assert.match(css,/\.fc-daygrid-day/);assert.match(css,/\.fc-toolbar/);assert.ok(files['dist/app.js'].length>100000);});

test('番茄钟标准包离线携带原字体和声音，声明固定计时能力并保留来源',()=>{const files=widgetFiles('pomodoro'),m=JSON.parse(new TextDecoder().decode(files['manifest.json'])),source=JSON.parse(readFileSync('components/widgetjs/pomodoro-source/SOURCE_FILES.json','utf8'));assert.deepEqual(m.capabilities,['storage.instance','focus.pomodoro','storage.shared','focus.pomodoro.window']);assert.equal(m.version,'0.3.0');assert.equal(source.upstreamId,'itime.fun.pomodoro');for(const item of source.files){const bytes=readFileSync('components/widgetjs/pomodoro-source/'+item.path);assert.equal(createHash('sha256').update(bytes).digest('hex'),item.sha256,item.path);if(/\.(ttf|wav)$/.test(item.path))assert.deepEqual(files['dist/'+item.path.split('/').at(-1)],new Uint8Array(bytes));}assert.match(new TextDecoder().decode(files['dist/style.css']),/@font-face/);});

test('生日本地包保留固定原气球及来源，且只声明实例存储能力',()=>{
  const files=widgetFiles('birthday-list'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json'])),provenance=JSON.parse(readFileSync('components/widgetjs/birthday-list-source/SOURCE_FILES.json','utf8'));
  assert.equal(createHash('sha256').update(files['dist/balloon-0HG1ynQp.png']).digest('hex'),'116ae6435fa85d6e1ef734d9003c5c5e5c1f5ef9ff0fa294e6ed365f12ff87fb');
  assert.deepEqual(manifest.capabilities,['storage.instance','storage.shared']);assert.equal(provenance.upstreamId,'itime.fun.birthday_list');
  for(const item of provenance.files)assert.equal(createHash('sha256').update(readFileSync('components/widgetjs/birthday-list-source/'+item.path)).digest('hex'),item.sha256,item.path);
});

test('组件配置请求使用已公布的方法并声明实例存储，试加载不掩盖正式权限缺失', () => {
  for (const name of portedWidgets) {
    const files=widgetFiles(name), manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
    let usesSettings=false;
    for(const [path,bytes]of Object.entries(files)) {
      if(!path.endsWith('.js'))continue;
      if(/\.request\(\s*['"]content\.live['"]/.test(new TextDecoder().decode(bytes)))assert.ok(manifest.capabilities.includes('content.live'),name+'缺少实时内容权限');
      if(/\.request\(\s*['"]system\.metrics['"]/.test(new TextDecoder().decode(bytes)))assert.ok(manifest.capabilities.includes('system.metrics'),name+'缺少本机监控权限');
      if(/['"]water\.(get|change)['"]/.test(new TextDecoder().decode(bytes)))assert.ok(manifest.capabilities.includes('reminders.water'),name+'缺少喝水提醒权限');
      for(const match of new TextDecoder().decode(bytes).matchAll(/\.request\(\s*['"](settings\.[a-z]+)['"]/g)) {
        usesSettings=true;
        assert.ok(['settings.get','settings.update'].includes(match[1]),name+'使用了未知配置方法：'+match[1]);
      }
    }
    if(usesSettings)assert.ok(manifest.capabilities.includes('storage.instance'),name+'缺少正式实例配置权限');
  }
});

test('主流虚拟币原14项都有随包的可用回退图与实时内容能力',()=>{
  const files=widgetFiles('coin-medium'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
  assert.equal(coins.length,14);assert.ok(manifest.capabilities.includes('content.live'));
  for(const coin of coins){const path='dist/'+(fallbacks[coin.ccy]||'bitcoin.png');assert.ok(files[path]?.length>100,path);assert.deepEqual([...files[path].slice(0,4)],[137,80,78,71]);}
});

test('仪表盘保留固定提交的三层原图片，并声明本机监控能力',()=>{
  const files=widgetFiles('dashboard'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
  assert.ok(manifest.capabilities.includes('system.metrics'));
  for(const [name,hash] of Object.entries({'hitler_body.png':'6dc0bc94c56c80ec5f54c2a43b8d6217903c0cd6','hitler_hand.png':'aedad82ebcf21f7b4d3437e2a4a63180c847b3b2','speedometer.png':'0a156ce6de98c99f443aa97e8c5d52a9bb019285'})){
    const bytes=files[posix.join(posix.dirname(manifest.entry),name)];assert.ok(bytes);
    assert.equal(createHash('sha1').update('blob '+bytes.length+'\0').update(bytes).digest('hex'),hash,name);
  }
});
test('思维导图保留固定提交的完整本地编辑源码，其余适配逐文件列明',()=>{
  const provenance=JSON.parse(readFileSync('components/widgetjs/mindmap-source/SOURCE_FILES.json','utf8'));
  assert.equal(provenance.revision,'9e8ff6ace455e7930117dec3816821ada51c48ab');
  const adapted=new Set(['editor/style.css','editor/mind-map/index.tsx','editor/mind-map/use-graph-init.ts','editor/mind-map/use-keyboard-shortcuts.ts','editor/mind-map/mind-map-toolbar.tsx']);
  let checked=0;
  for(const item of provenance.files){const path=item.target.replaceAll('\\','/');if(!path.startsWith('editor/')||adapted.has(path))continue;assert.equal(createHash('sha256').update(readFileSync('components/widgetjs/mindmap-source/'+path)).digest('hex'),item.sha256,path);checked++;}
  assert.ok(checked>=20);
  const files=widgetFiles('mindmap'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
  for(const capability of ['storage.instance','storage.document','documents.open','documents.save','documents.drop'])assert.ok(manifest.capabilities.includes(capability));
});
test('服务器监控随包交付原平台与系统图标，并声明固定监控及复制能力',()=>{
  const provenance=JSON.parse(readFileSync('components/widgetjs/server-source/SOURCE_FILES.json','utf8')),files=widgetFiles('server'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
  assert.equal(provenance.revision,'e8cba77deb4ba7beb1e35939601b912bbe6aadab');let checked=0;
  for(const item of provenance.files){if(!item.target.endsWith('.png'))continue;const name=posix.basename(item.target.replaceAll('\\','/'));if(files['dist/'+name]){assert.equal(createHash('sha256').update(files['dist/'+name]).digest('hex'),item.sha256,name);checked++;}}
  assert.equal(checked,17);for(const cap of ['storage.instance','servers.monitor','clipboard.write'])assert.ok(manifest.capabilities.includes(cap));
});
test('打工进度随包交付11张原表情及原许可，声明核心倒计时能力',()=>{
  const provenance=JSON.parse(readFileSync('components/widgetjs/labor-source/SOURCE_FILES.json','utf8')),files=widgetFiles('labor'),manifest=JSON.parse(new TextDecoder().decode(files['manifest.json']));
  assert.equal(provenance.revision,'714802e66336133396ab55463eb5a26d08fc0788');let checked=0;
  for(const item of provenance.files){if(!item.target.endsWith('.png'))continue;const name=posix.basename(item.target.replaceAll('\\','/'));assert.equal(createHash('sha256').update(files['dist/'+name]).digest('hex'),item.sha256,name);checked++;}assert.equal(checked,11);assert.match(new TextDecoder().decode(files['dist/LICENSE.txt']),/GNU GENERAL PUBLIC LICENSE/);assert.ok(manifest.capabilities.includes('reminders.labor'));
});
