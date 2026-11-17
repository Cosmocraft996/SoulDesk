import test from 'node:test';
import assert from 'node:assert/strict';
import { drawCoinChart } from './coin-chart.js';
import { createCoin } from './coin-model.js';

// 记录实际绘图函数发出的画布命令，不冒充WebView像素或桌面验收。
function canvas(width = 100, height = 80) {
  const calls = [], context = {};
  for (const method of ['scale', 'clearRect', 'beginPath', 'moveTo', 'lineTo', 'closePath', 'stroke', 'fill', 'arc']) {
    context[method] = (...args) => calls.push([method, ...args]);
  }
  context.createLinearGradient = (...args) => {
    calls.push(['gradient', ...args]);
    return { addColorStop: (...args) => calls.push(['colorStop', ...args]) };
  };
  return { calls, getContext: () => context, getBoundingClientRect: () => ({ width, height }) };
}

test('单个真实采样或同一时间重复报价绘制一个可见圆点，不伪造第二个历史点', async () => {
  const model = createCoin(async () => ({ data: { instrument: 'ETH-USD', price: '1811.65', timestamp: 1000, reference: '1851.37', basis: 'open24h' } }));
  await model.select('ETH-USD');
  await model.refresh();
  assert.equal(model.state.points.length, 1);
  const target = canvas();
  drawCoinChart(target, model.state.points.map(point => point.price), '#5fc25d', 2);
  const circles = target.calls.filter(call => call[0] === 'arc');
  assert.equal(circles.length, 1);
  const [, x, y, radius] = circles[0];
  assert.ok(radius > 0 && x - radius >= 0 && x + radius <= 100 && y - radius >= 0 && y + radius <= 80);
  assert.equal(target.calls.filter(call => call[0] === 'fill').length, 1);
  assert.equal(target.calls.some(call => call[0] === 'lineTo'), false);
  assert.equal(model.state.points.length, 1);
  model.dispose();
});

test('多个真实采样仍绘制原折线和渐隐填充', () => {
  const target = canvas();
  drawCoinChart(target, ['100', '102'], '#f82842', 1);
  assert.ok(target.calls.some(call => call[0] === 'stroke'));
  assert.ok(target.calls.some(call => call[0] === 'lineTo'));
  assert.deepEqual(target.calls.filter(call => call[0] === 'colorStop'), [['colorStop', 0, '#f828423d'], ['colorStop', 1, '#f8284200']]);
  assert.equal(target.calls.some(call => call[0] === 'arc'), false);
});

test('没有真实采样或画布尚无可用尺寸时不生成行情图形', () => {
  for (const [target, values] of [[canvas(), []], [canvas(0, 80), ['100']], [canvas(100, 8), ['100']]]) {
    drawCoinChart(target, values, '#5fc25d', 1);
    assert.equal(target.calls.some(call => ['stroke', 'fill', 'arc'].includes(call[0])), false);
  }
});
