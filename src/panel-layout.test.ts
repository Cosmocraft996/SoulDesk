import test from 'node:test';
import assert from 'node:assert/strict';
import { changeLayout, fitLayout, initialLayouts, type SnapMatch } from './panel-layout.ts';
const min = { width: 180, height: 100 };

test('吸附反馈标识双方实际参与的边，脱离和原地点击清空反馈', () => {
  const start = { x: 20, y: 20, width: 200, height: 180 };
  const peers = [{ x: 300, y: 250, width: 200, height: 180 }];
  const matches: SnapMatch[] = [];
  const expected: SnapMatch[] = [{ edge: 'right', peer: 0, peerEdge: 'left' }, { edge: 'bottom', peer: 0, peerEdge: 'top' }];
  changeLayout(start, 74, 54, false, 800, min, peers, undefined, matches);
  assert.deepEqual(matches, expected);
  changeLayout(start, 74, 44, true, 800, min, peers, undefined, matches);
  assert.deepEqual(matches, expected);
  changeLayout(start, 68, 40, false, 800, min, peers, undefined, matches);
  assert.deepEqual(matches, []);
  changeLayout(start, -14, 20, false, 800, min, peers, undefined, matches);
  assert.deepEqual(matches, [{ edge: 'left', peer: undefined, peerEdge: 'left' }]);
  changeLayout(start, 0, 0, false, 800, min, peers, undefined, matches);
  assert.deepEqual(matches, []);
});
test('拖动和缩放不能越过面板边界或压坏组件最小尺寸', () => {
  const start = { x: 20, y: 20, width: 200, height: 180 };
  assert.deepEqual(changeLayout(start, 999, -999, false, 400, min), { x: 200, y: 0, width: 200, height: 180 });
  assert.deepEqual(changeLayout(start, -999, -999, true, 400, min), { x: 20, y: 20, width: 180, height: 100 });
  assert.deepEqual(changeLayout(start, 999, 0, true, 400, min), { ...start, width: 380 });
  assert.equal(fitLayout(start, 150, min).width, 180);
});

test('拖动吸附组件同侧或相邻边缘，超出8像素即可脱离，不累积吸附偏移', () => {
  const start = { x: 20, y: 20, width: 200, height: 180 };
  const peers = [{ x: 300, y: 250, width: 200, height: 180 }];
  assert.deepEqual(changeLayout(start, 74, 54, false, 800, min, peers), { ...start, x: 100, y: 70 });
  assert.deepEqual(changeLayout(start, 274, 224, false, 800, min, peers), { ...start, x: 300, y: 250 });
  assert.deepEqual(changeLayout(start, 68, 40, false, 800, min, peers), { ...start, x: 88, y: 60 });
  assert.deepEqual(changeLayout(start, 0, 0, false, 800, min, peers), start);
});

test('面板边界及滚动后的可见上下边界参与吸附，纵向仍可自由扩展', () => {
  const start = { x: 20, y: 20, width: 200, height: 180 };
  assert.deepEqual(changeLayout(start, -13, -13, false, 800, min), { ...start, x: 0, y: 0 });
  assert.equal(changeLayout(start, 574, 10, false, 800, min).x, 600);
  assert.equal(changeLayout(start, 20, 194, false, 800, min, [], { top: 100, bottom: 400 }).y, 220);
  assert.equal(changeLayout(start, 20, 74, false, 800, min, [], { top: 100, bottom: 400 }).y, 100);
  assert.equal(changeLayout(start, 20, 700, false, 800, min, [], { top: 0, bottom: 400 }).y, 720);
});

test('缩放吸附右边和下边，保持左上角，不突破最小尺寸和画布边界', () => {
  const start = { x: 20, y: 20, width: 200, height: 180 };
  const peers = [{ x: 300, y: 250, width: 200, height: 180 }];
  assert.deepEqual(changeLayout(start, 74, 44, true, 800, min, peers), { ...start, width: 280, height: 230 });
  assert.deepEqual(changeLayout(start, 274, 224, true, 800, min, peers), { ...start, width: 480, height: 410 });
  assert.deepEqual(changeLayout(start, 999, -999, true, 400, min, [{ x: 195, y: 115, width: 1, height: 1 }]), { ...start, width: 380, height: 100 });
  assert.equal(changeLayout(start, 10, 194, true, 800, min, [], { top: 0, bottom: 400 }).height, 380);
});

test('多个吸附目标取最近值，等距优先原顺序，极窄面板保留最小宽度', () => {
  const start = { x: 0, y: 20, width: 200, height: 180 };
  const peers = [{ x: 300, y: 500, width: 200, height: 180 }, { x: 304, y: 700, width: 200, height: 180 }];
  assert.equal(changeLayout(start, 302, 10, false, 800, min, peers).x, 300);
  assert.equal(changeLayout(start, 303, 10, false, 800, min, peers).x, 304);
  assert.equal(changeLayout(start, 300, 10, false, 800, min, peers).x, 300);
  assert.deepEqual(changeLayout(start, 999, 10, true, 150, min), { ...start, width: 180, height: 190 });
});
test('面板尺寸与桌面尺寸独立，显示器变窄不覆盖已保存布局', () => {
  const saved = { clock: { x: 120, y: 80, width: 400, height: 240 } };
  const instances = [{ id: 'clock', packageId: 'clock', width: 700, height: 500 }];
  const packages = [{ id: 'clock', defaultSize: { width: 300, height: 200 }, minSize: min }];
  assert.equal(initialLayouts(instances, packages, saved, 300).clock.width, 300);
  assert.deepEqual(initialLayouts(instances, packages, saved, 600).clock, saved.clock);
  assert.equal(instances[0].width, 700);
});
