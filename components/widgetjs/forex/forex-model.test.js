import test from 'node:test';
import assert from 'node:assert/strict';
const imported = await import('./forex-model.js').catch(() => null);

test('金额使用精确十进制乘除，不丢大整数，按八位小数四舍五入', () => {
  assert.ok(imported, '精确汇率换算尚未实现');
  const { convertAmount } = imported;
  assert.equal(convertAmount('0.1', '0.2'), '0.02');
  assert.equal(convertAmount('9007199254740993', '7.12345678'), '64162394599995673.72978254');
  assert.equal(convertAmount('1', '3', true), '0.33333333');
  assert.equal(convertAmount('2', '3', true), '0.66666667');
  assert.equal(convertAmount('0', '6.8'), '0');
  assert.equal(convertAmount('100', '1'), '100');
  assert.equal(convertAmount('0.000000005', '1'), '0.00000001');
  assert.throws(() => convertAmount('1', '0'), /汇率/);
  for (const invalid of ['NaN', 'Infinity', '1e8', '-1', '1,000', '', '1.2.3']) assert.throws(() => convertAmount(invalid, '1'));
});

test('同币种对刷新失败保留上次汇率，换币种后失败不能沿用旧币种价格', async () => {
  assert.ok(imported);
  let fail = false;
  const rates = imported.createRates(async () => {
    if (fail) throw new Error('源不可用');
    return { ResultCode: '0', Result: { revCode: [{ price: '7.20', list: [] }] } };
  });
  await rates.refresh('USD', 'CNY'); fail = true;
  await rates.refresh('USD', 'CNY');
  assert.equal(rates.state.rate.price, '7.20');
  assert.match(rates.state.error, /源不可用/);
  await rates.refresh('EUR', 'JPY');
  assert.equal(rates.state.rate, null);
});

test('汇率源必须成功且含正十进制价格；拒绝缺失值和异常值', () => {
  assert.ok(imported, '汇率响应校验尚未实现');
  const { readRate } = imported;
  assert.equal(readRate({ ResultCode: '0', Result: { revCode: [{ price: '7.20', list: [] }] } }).price, '7.20');
  for (const response of [{}, { ResultCode: '1' }, { ResultCode: '0', Result: { revCode: [{ price: 'NaN' }] } }, { ResultCode: '0', Result: { revCode: [{ price: '0' }] } }]) assert.throws(() => readRate(response));
});

test('旧币种请求晚到时不能覆盖最新币种对应的汇率', async () => {
  assert.ok(imported, '汇率请求隔离尚未实现');
  const { createRates } = imported;
  const waiting = new Map();
  const rates = createRates(pair => new Promise(resolve => waiting.set(pair.join('/'), resolve)));
  const first = rates.refresh('USD', 'CNY');
  const second = rates.refresh('EUR', 'JPY');
  waiting.get('EUR/JPY')({ ResultCode: '0', Result: { revCode: [{ price: '165.25', list: [] }] } });
  await second;
  waiting.get('USD/CNY')({ ResultCode: '0', Result: { revCode: [{ price: '7', list: [] }] } });
  await first;
  assert.equal(rates.state.rate.price, '165.25');
  assert.deepEqual(rates.state.pair, ['EUR', 'JPY']);
});
