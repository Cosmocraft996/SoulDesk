import test from 'node:test';
import assert from 'node:assert/strict';
const imported = await import('./widget-permissions.ts').catch(()=>null);
test('AI图形正式实例仅允许用户手势全屏，预览不授予额外权限',()=>{
  assert.ok(imported);const permissions=imported.framePermissions(['ai.generate'],false);assert.equal(permissions.allow,'fullscreen');assert.equal(permissions.allowFullScreen,true);assert.equal(permissions.sandbox,'allow-scripts allow-same-origin allow-forms');assert.deepEqual(imported.framePermissions(['ai.generate'],true),imported.framePermissions([],false));
  assert.equal(imported.framePermissions(['jobs.manage'],false).allow,'fullscreen');assert.equal(imported.framePermissions(['jobs.manage'],false).allowFullScreen,true);assert.deepEqual(imported.framePermissions(['jobs.manage'],true),imported.framePermissions([],false));
});

test('专用灵动通知仅在正式实例声明后允许音频自动播放，预览继续禁用',()=>{
  assert.ok(imported);
  assert.equal(imported.framePermissions(['notifications.island'],false).allow,'autoplay');
  assert.equal(imported.framePermissions(['notifications.island'],true).allow,undefined);
  assert.equal(imported.framePermissions(['reminders.phone'],false).allow,undefined);
});

test('普通组件保留基础沙箱，网页嵌入和剪贴板按声明分别开放',()=>{
  assert.ok(imported,'组件浏览器权限映射尚未实现');
  const base=imported.framePermissions([],false);
  assert.equal(base.sandbox,'allow-scripts allow-same-origin allow-forms');
  assert.equal(base.allow,undefined);assert.equal(base.allowFullScreen,false);
  const embed=imported.framePermissions(['web.embed'],false);
  assert.equal(embed.allow,'fullscreen');assert.equal(embed.allowFullScreen,true);
  assert.ok(embed.sandbox.includes('allow-presentation'));
  assert.ok(!embed.sandbox.includes('allow-top-navigation'));
  assert.ok(!embed.sandbox.includes('allow-popups'));
  const clipboard=imported.framePermissions(['clipboard.read','clipboard.write'],false);
  assert.equal(clipboard.allow,'clipboard-read; clipboard-write');
  assert.equal(clipboard.allowFullScreen,false);
});

test('隐藏试加载不授予浏览器剪贴板和全屏权限',()=>{
  assert.ok(imported);
  assert.deepEqual(imported.framePermissions(['web.embed','clipboard.read','clipboard.write'],true),imported.framePermissions([],false));
});
