import assert from 'node:assert/strict';
import test from 'node:test';
import { widgetEntryUrl } from './widget-url.ts';

test('Windows 组件入口保留目录层次，模块和 CSS 不落到协议根目录', () => {
  // Tauri 2.12.5 的 convertFileSrc 会整体 encodeURIComponent 路径。
  const url = widgetEntryUrl('http://widget.localhost/local.example.clock%2Fdist%2Findex.html');
  assert.equal(new URL('./clock.js', url).href, 'http://widget.localhost/local.example.clock/dist/clock.js');
  assert.equal(new URL('./style.css', url).href, 'http://widget.localhost/local.example.clock/dist/style.css');
});
