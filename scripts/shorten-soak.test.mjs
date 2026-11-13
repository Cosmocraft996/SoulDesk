import test from 'node:test';
import assert from 'node:assert/strict';
import { assessAdjustedRun } from './shorten-soak.mjs';
function fixture(seconds = 1800) {
  const rows = Array.from({ length: Math.floor(seconds / 30) + 1 }, (_, i) => ({ elapsedSeconds: i * 30, events: 5, views: 0, sessions: 0, temporarySubscriptions: 0, windowTasks: 0, trials: 0, streamCount: 62, queueOverflows: 0, healthy: true, process: { privateBytes: 12 * 1048576, workingSetBytes: 40 * 1048576, cpuPercentOneCore: 2 } }));
  return [{ phase: 'g3-soak', elapsedSeconds: seconds, latestSample: rows.at(-1), healthObservationGapsOver3s: 0, samplingGapsOver90s: 0 }, rows];
}
test('累计不足30分钟不能通过，满30分钟不能冒充24小时', () => {
  assert.throws(() => assessAdjustedRun(...fixture(1770)));
  const result = assessAdjustedRun(...fixture());
  assert.equal(result.passed, true); assert.equal(result.effectiveTargetSeconds, 1800); assert.equal(result.full24Hours, false);
});
test('数据中断、资源边界或采样缺口不能因缩短时长而忽略', () => {
  for (const mutate of [(r) => { r.failure = '故障'; }, (r) => { r.healthObservationGapsOver3s = 1; }, (r, rows) => { rows[30].events = 6; }, (r, rows) => { rows[30].queueOverflows = 1; }, (r, rows) => { rows.splice(20, 4); }]) { const [r, rows] = fixture(); mutate(r, rows); assert.throws(() => assessAdjustedRun(r, rows)); }
});
