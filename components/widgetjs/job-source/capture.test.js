import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { createHash } from 'node:crypto';
import test from 'node:test';
import assert from 'node:assert/strict';

const capture = readFileSync(new URL('./capture.js', import.meta.url), 'utf8');
const login = readFileSync(new URL('./login.js', import.meta.url), 'utf8');
function fixture(overrides = {}) {
  const requested = [];
  const state = { jobList: [{ encryptJobId: 'id1', jobName: '前端工程师', skills: ['Vue'], uid: '账号不可泄露', cookie: '不可泄露', analysis: '站点附加字段' }], pageVo: { page: 1, pageSize: 30 }, jobTotal: 1 };
  return { requested, state, context: { location: { origin: 'https://www.zhipin.com', pathname: '/web/geek/job' }, window: { _PAGE: { uid: '账号不可泄露', other: '不可泄露' } }, document: { querySelector(selector) { requested.push(selector); return { __vue__: state }; }, get cookie() { throw Error('不得读取Cookie'); } }, ...overrides } };
}
const evaluate = (script, context) => JSON.parse(JSON.stringify(runInNewContext(script, context, { timeout: 100 })));

test('固定采集只读取原职位结构白名单，不返回账号和站点额外字段', () => {
  const { context, requested } = fixture();
  const result = evaluate(capture, context);
  assert.deepEqual(requested, ['.page-job-wrapper']);
  assert.equal(result.jobs.length, 1); assert.equal(Object.keys(result.jobs[0]).length, 18);
  assert.equal(result.jobs[0].encryptJobId, 'id1'); assert.deepEqual(result.jobs[0].skills, ['Vue']);
  assert.equal(result.jobs[0].brandLogo, null); assert.equal(result.loggedIn, true);
  assert.doesNotMatch(JSON.stringify(result), /不可泄露|站点附加字段|cookie|uid/);
});
test('固定采集拒绝错域、错路径和未登录，不读取职位页面', () => {
  for (const location of [{ origin: 'https://login.zhipin.com', pathname: '/web/geek/job' }, { origin: 'https://www.zhipin.com', pathname: '/job_detail/id1.html' }]) {
    const f = fixture({ location }); assert.deepEqual(evaluate(capture, f.context), { error: 'wrong_page' }); assert.deepEqual(f.requested, []);
  }
  const f = fixture({ window: {} }); assert.deepEqual(evaluate(capture, f.context), { error: 'login_required' }); assert.deepEqual(f.requested, []);
});
test('固定采集将未载入结构标为未就绪', () => {
  const f = fixture(); f.state.jobList = undefined;
  assert.deepEqual(evaluate(capture, f.context), { error: 'not_ready' });
  f.state.jobList = []; f.state.pageVo = undefined;
  assert.deepEqual(evaluate(capture, f.context), { error: 'not_ready' });
});
test('登录检查只返回固定域和布尔状态', () => {
  const f = fixture(); assert.deepEqual(evaluate(login, f.context), { origin: 'https://www.zhipin.com', loggedIn: true });
  f.context.location.origin = 'https://evil.example'; assert.equal(evaluate(login, f.context).loggedIn, false);
  f.context.location.origin = 'https://www.zhipin.com'; f.context.window = {}; assert.equal(evaluate(login, f.context).loggedIn, false);
});
test('异常字段和读取故障返回有限错误，不序列化站点对象', () => {
  for (const bad of [null, { encryptJobId: 'id1', jobName: { toJSON() { throw Error('不得执行'); } } }, { encryptJobId: 'id1', jobName: '开发', skills: [{ uid: '不可泄露' }] }]) {
    const f = fixture(); f.state.jobList = [bad]; assert.deepEqual(evaluate(capture, f.context), { error: 'invalid_data' });
  }
  const f = fixture(); f.state.jobList[0] = Object.defineProperty({}, 'encryptJobId', { get() { throw Error('敏感异常不可泄露'); } });
  assert.deepEqual(evaluate(capture, f.context), { error: 'capture_failed' });
  f.state.jobList = Array(101).fill({}); assert.deepEqual(evaluate(capture, f.context), { error: 'capacity_exceeded' });
  f.state.jobList = []; f.state.jobTotal = { uid: '不可泄露' }; assert.deepEqual(evaluate(capture, f.context), { error: 'invalid_metadata' });
  f.context.window = Object.defineProperty({}, '_PAGE', { get() { throw Error('敏感异常不可泄露'); } });
  assert.deepEqual(evaluate(login, f.context), { error: 'not_ready' });
});
test('31份固定来源保持原字节，入口散列与静态提示词清单一致', () => {
  const source = JSON.parse(readFileSync(new URL('./SOURCE_FILES.json', import.meta.url), 'utf8'));
  assert.equal(source.revision, '58806a7f80ab5389b49087dd66498bdd7c1982c2578cb199ead31b4e27d3d11f');
  assert.equal(source.files.length, 31);
  let total = 0;
  for (const file of source.files) {
    assert.match(file.path, /^original\/assets\/[\w.-]+$/);
    const bytes = readFileSync(new URL(file.path, import.meta.url));
    assert.equal(bytes.length, file.bytes); total += bytes.length;
    assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256);
    assert.ok(file.url.startsWith('https://widgetjs.cn/ai/job/assets/'));
  }
  assert.equal(total, 3875098);
  const prompts = JSON.parse(readFileSync(new URL('./prompts.json', import.meta.url), 'utf8'));
  assert.deepEqual(Object.keys(prompts).sort(), ['mindMap', 'wiki', 'interviewer', 'knowledgeWordOptimization', 'jobDetailAnalysis', 'jobAnalysisExample', 'jobAnalysis'].sort());
  assert.match(prompts.jobAnalysis, /encryptJobId/);
});
test('未完整载入的页面和过大结果不会返回部分岗位', () => {
  const f = fixture(); f.state.jobTotal = 31;
  assert.deepEqual(evaluate(capture, f.context), { error: 'not_ready' });
  f.state.jobTotal = 30;
  f.state.jobList = Array.from({ length: 30 }, (_, n) => ({ encryptJobId: 'id' + n, jobName: '开发', skills: Array(100).fill('字'.repeat(300)) }));
  assert.deepEqual(evaluate(capture, f.context), { error: 'capacity_exceeded' });
});
