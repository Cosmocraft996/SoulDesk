import { readFileSync, writeFileSync, readdirSync, mkdirSync, copyFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { spawnSync } from 'node:child_process';
import ts from 'typescript';
import { hotspotVariants } from './widgetjs-hotspots.mjs';
const upstream = 'artifacts/widgetjs-upstream/hotspot';
const revision = 'cff331e104d8f991e5cd7aa1be164d0fb8c93cc9';
const result = spawnSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding: 'utf8' });
if (result.status || result.stdout.trim() !== revision) throw new Error('热点源码不匹配已审计提交');
const target = 'components/widgetjs/hotspot-source';
function protectFunctions(script, names) {
  const file = ts.createSourceFile('component.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const edits = [];
  for (const node of file.statements) if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text) && node.body) {
    edits.push([node.body.getStart(file) + 1, '\ntry {\n']);
    edits.push([node.body.end - 1, '\n} catch (error) { contentFailure(error); }\n']);
  }
  for (const [position, text] of edits.sort((a, b) => b[0] - a[0])) script = script.slice(0, position) + text + script.slice(position);
  return script;
}
function transform(text, file) {
  text = text.replaceAll("from 'axios'", "from '@/client'").replace(/\baxios\b/g, 'client')
    .replaceAll("from '@widget-js/vue3'", "from '@/native'").replaceAll("from '@widget-js/core'", "from '@/native'").replaceAll("from 'element-plus'", "from '@/native'")
    .replace(/\bWidgetWrapper\b/g, 'ComponentFrame').replace(/\buseWidgetSize\b/g, 'useComponentSize').replace(/\buseWidget\b/g, 'useComponent').replace(/\bElScrollbar\b/g, 'ScrollArea')
    .replace(/<\/?widget-wrapper/g, match => match.replace('widget-wrapper', 'ComponentFrame'))
    .replace(/<\/?el-scrollbar/g, match => match.replace('el-scrollbar', 'ScrollArea'))
    .replace(/\bBrowserWindowApi\.openUrl/g, 'openExternal').replace(/\bBrowserWindowApi\b/g, 'openExternal')
    .replace(/WidgetParams\.fromCurrentLocation\(\)/g, 'currentViewport()').replace(/\bWidgetParams\b/g, 'currentViewport')
    .replaceAll('var(widget-background-color)', 'var(--widget-background-color)');
  if (file.endsWith('.vue')) {
    text = text.replace(/:src="([^"]+)"/g, ':src="secureImage($1)"');
    if (file.endsWith('BangumiItem.vue')) text = text.replace(/<el-avatar\s+:id="([^"]+)"\s+shape="square"\s+size="large"\s+:src="([^"]+)"\s*\/>/, '<div :id="$1" class="bangumi-cover"><img crossorigin="anonymous" :src="$2" alt="追番封面"></div>');
    text = text.replace(/<script([^>]*)>([\s\S]*?)<\/script>/, (_all, attrs, script) => {
      if (file.endsWith('BangumiWidget.vue')) script = script.replace('function refresh()', 'async function refresh()').replace("client.get('https://bangumi", "await client.get('https://bangumi");
      if (file.endsWith('BangumiItem.vue')) script = script.replace('function getImgColor(img) {', 'function getImgColor(img) {\nif (!img?.naturalWidth || img.dataset.failedRemote) return;');
      script = protectFunctions(script, ['getHotList', 'refresh', 'getImgColor']);
      script = "\nimport { contentFailure } from '@/native';\n" + script;
      if (text.includes('secureImage(')) script = "\nimport { secureImage } from '@/native';\n" + script;
      if (text.includes('<ComponentFrame') && !/import\s*\{[^}]*ComponentFrame/s.test(script)) script = "\nimport { ComponentFrame } from '@/native';\n" + script;
      if (text.includes('<ScrollArea') && !/import\s*\{[^}]*ScrollArea/s.test(script)) script = "\nimport { ScrollArea } from '@/native';\n" + script;
      return '<script' + attrs + '>' + script + '</script>';
    });
    text = text.replace(/<style(?![^>]*\bscoped\b)([^>]*)>/g, '<style scoped$1>');
    if (text.includes('darken(') || text.includes('lighten(')) {
      text = text.replace(/<style([^>]*)>/g, '<style$1>\n@use "sass:color";')
        .replace(/darken\(([^,]+),\s*([\d.]+)%\)/g, 'color.adjust($1, $lightness: -$2%)')
        .replace(/lighten\(([^,]+),\s*([\d.]+)%\)/g, 'color.adjust($1, $lightness: $2%)');
    }
  }
  return text;
}
function copyTree(source, out) {
  mkdirSync(out, { recursive: true });
  for (const item of readdirSync(source, { withFileTypes: true })) {
    const from = join(source, item.name), to = join(out, item.name);
    if (item.isDirectory()) copyTree(from, to);
    else if (!/ConfigView\.vue$|Routes\.ts$|\.widget\.ts$|widget-router\.ts$/.test(item.name) && /\.(vue|ts|png|svg|jpg|jpeg)$/.test(item.name)) {
      mkdirSync(dirname(to), { recursive: true });
      if (/\.(vue|ts)$/.test(item.name)) writeFileSync(to, transform(readFileSync(from, 'utf8'), relative(upstream, from)));
      else copyFileSync(from, to);
    }
  }
}
copyTree(join(upstream, 'src/widgets'), join(target, 'widgets'));
copyTree(join(upstream, 'src/assets'), join(target, 'assets'));
mkdirSync(join(target, 'api'), { recursive: true });
writeFileSync(join(target, 'api/BilibiliApi.ts'), transform(readFileSync(join(upstream, 'src/api/BilibiliApi.ts'), 'utf8'), 'api/BilibiliApi.ts'));
copyFileSync(join(upstream, 'LICENSE'), join(target, 'LICENSE.txt'));
const catalog = JSON.parse(readFileSync('docs/widgetjs-catalog.json', 'utf8'));
for (const [name] of hotspotVariants) {
  const entry = catalog.items.find(item => item.sourceRepository === 'widget-js/hotspot' && item.path.split('/').at(-1).replaceAll('_', '-') === name);
  if (!entry) throw new Error('目录缺少热点项：' + name);
  const directory = 'components/widgetjs/hotspot-' + name;
  mkdirSync(directory, { recursive: true });
  const manifest = { schemaVersion: 1, id: 'local.souldesk.widgetjs.hotspot-' + name, name: entry.title, version: '0.1.0', hostApiMajor: 1, loadCheck: 1, entry: 'dist/index.html', defaultSize: { width: name === 'world-cup' ? 520 : 360, height: 420 }, minSize: { width: 260, height: 220 }, capabilities: ['storage.instance', 'content.read', 'links.open'], settingsSchema: { type: 'object', additionalProperties: false, required: ['background', 'foreground', 'radius'], properties: { background: { type: 'string', title: '背景颜色', maxLength: 7, default: '#faf7f2' }, foreground: { type: 'string', title: '文字颜色', maxLength: 7, default: '#494644' }, radius: { type: 'integer', title: '圆角', minimum: 0, maximum: 40, default: 16 } } } };
  writeFileSync(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
  writeFileSync(join(directory, 'index.html'), '<!doctype html>\n<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>' + entry.title + '</title><link rel="stylesheet" href="./style.css"></head><body data-variant="' + name + '"><div id="app"></div><script type="module" src="./app.js"></script></body></html>\n');
  writeFileSync(join(directory, 'SOURCE.txt'), '来源：https://github.com/widget-js/hotspot\n固定提交：' + revision + '\n原始标识：' + entry.upstreamId + '\n保留对应widgets目录、数据解析、排序/日期选择、点击打开和刷新周期；改为SoulDesk受控内容/链接接口、独立主题设置和原生滚动容器，保留原图片与追番封面取色。\n根LICENSE为GPL v2，package另写MIT；本机自用，保留根许可证与源码事实。预览只呈现空数据并明确不联网；内容源拒绝访问或格式变化会提示失败，不代替为空榜，不引入登录或Cookie。\n');
}
console.log('已移植9个热点组件的固定源码；尚需构建和运行验收。');
