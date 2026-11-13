import test from 'node:test';
import assert from 'node:assert/strict';
import { changeLayout, fitLayout, initialLayouts } from './panel-layout.ts';
const min = { width: 180, height: 100 };
test('拖动和缩放不能越过面板边界或压坏组件最小尺寸', () => {
  const start = { x: 20, y: 20, width: 200, height: 180 };
  assert.deepEqual(changeLayout(start, 999, -999, false, 400, min), { x: 200, y: 0, width: 200, height: 180 });
  assert.deepEqual(changeLayout(start, -999, -999, true, 400, min), { x: 20, y: 20, width: 180, height: 100 });
  assert.equal(changeLayout(start, 999, 0, true, 400, min).x, 0);
  assert.equal(fitLayout(start, 150, min).width, 180);
});
test('面板尺寸与桌面尺寸独立，显示器变窄不覆盖已保存布局', () => {
  const saved = { clock: { x: 120, y: 80, width: 400, height: 240 } };
  const instances = [{ id: 'clock', packageId: 'clock', width: 700, height: 500 }];
  const packages = [{ id: 'clock', defaultSize: { width: 300, height: 200 }, minSize: min }];
  assert.equal(initialLayouts(instances, packages, saved, 300).clock.width, 300);
  assert.deepEqual(initialLayouts(instances, packages, saved, 600).clock, saved.clock);
  assert.equal(instances[0].width, 700);
});
