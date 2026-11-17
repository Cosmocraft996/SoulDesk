import test from 'node:test';
import assert from 'node:assert/strict';

const module = await import('./note-model.js').catch(() => null);
function model() { assert.ok(module, '便签持久化模型尚未实现'); return module; }
function deferred() { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }

test('快速连续编辑串行写入，旧响应不覆盖新文本，全部写入后才显示已保存', async () => {
  const { createNote } = model();
  const calls = [], first = deferred();
  const note = createNote({}, async value => { calls.push(value); if (calls.length === 1) await first.promise; });
  const pending = note.edit({ text: '第一版' });
  note.edit({ text: '最新版本' });
  assert.equal(note.state.status, 'saving');
  assert.equal(calls.length, 1);
  first.resolve();
  await pending;
  assert.deepEqual(calls.map(value => value.text), ['第一版', '最新版本']);
  assert.equal(note.state.value.text, '最新版本');
  assert.equal(note.state.status, 'saved');
});

test('保存失败保留未保存内容，重试写入最新版本且不丢已有扩展字段', async () => {
  const { createNote } = model();
  const calls = [];
  const note = createNote({ text: '旧文', fontSize: 24, other: { retained: true } }, async value => {
    calls.push(value); if (calls.length === 1) throw new Error('磁盘写入失败');
  });
  await note.edit({ text: '待保存' });
  assert.equal(note.state.status, 'error');
  assert.match(note.state.error, /磁盘/);
  assert.equal(note.state.value.text, '待保存');
  await note.retry();
  assert.equal(note.state.status, 'saved');
  assert.deepEqual(calls[1].other, { retained: true });
});

test('空库仅给出说明而不主动写入；坏存储明确拒绝而不覆盖内容', () => {
  const { createNote } = model();
  let writes = 0;
  const note = createNote({}, () => { writes++; });
  assert.match(note.state.value.text, /Ctrl/);
  assert.equal(note.state.value.fontSize, 20);
  assert.equal(writes, 0);
  for (const stored of [null, [], { text: 1 }, { fontSize: 9 }, { fontSize: NaN }]) assert.throws(() => createNote(stored, () => {}), /便签/);
});

test('字号限制10至120，超出存储字节预算时保留现有草稿并拒绝本次修改', async () => {
  const { createNote, fontAfterWheel } = model();
  assert.equal(fontAfterWheel(20, -1), 21);
  assert.equal(fontAfterWheel(10, 20), 10);
  assert.equal(fontAfterWheel(120, -1), 120);
  assert.equal(fontAfterWheel(25, 0), 25);
  const note = createNote({ text: '草稿' }, async () => {});
  assert.throws(() => note.edit({ text: '中'.repeat(1_400_000) }), /4 MiB/);
  assert.equal(note.state.value.text, '草稿');
  await note.edit({ text: '中'.repeat(100_000) });
  assert.equal(note.state.status, 'saved');
});
