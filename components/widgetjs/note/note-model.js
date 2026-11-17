// 从 WidgetJS 便签移植；写入由 SoulDesk 按实例持久化，不使用网页本地存储。
const instructions = '便签使用说明：\n1. 使用 Ctrl + 鼠标滚轮 调整字体大小。\n2. 内容会自动保存，离开前请确认下方显示“已保存”。\n3. 适合快速记录临时信息。';
const limit = 4 * 1024 * 1024;

function validate(value) {
  if (!value || Array.isArray(value) || typeof value !== 'object' || typeof value.text !== 'string' || !Number.isInteger(value.fontSize) || value.fontSize < 10 || value.fontSize > 120) throw new Error('便签存储格式无效，请保留原数据后检查');
  if (new TextEncoder().encode(JSON.stringify(value)).length > limit) throw new Error('便签内容超过 4 MiB，本次修改未保存，请缩短内容或复制到文件');
  return value;
}

export function fontAfterWheel(current, delta) {
  return Math.min(120, Math.max(10, current + (delta < 0 ? 1 : delta > 0 ? -1 : 0)));
}

export function createNote(stored, persist, changed = () => {}) {
  if (!stored || Array.isArray(stored) || typeof stored !== 'object') throw new Error('便签存储格式无效');
  const state = { value: validate({ text: instructions, fontSize: 20, ...structuredClone(stored) }), status: 'saved', error: '' };
  let revision = 0, saved = 0, running = false, pending;
  function announce() { changed(state); }
  async function drain() {
    while (saved !== revision) {
      const writing = revision, value = structuredClone(state.value);
      state.status = 'saving'; state.error = ''; announce();
      try { await persist(value); }
      catch (error) { state.status = 'error'; state.error = String(error?.message || error); announce(); return; }
      saved = writing;
    }
    state.status = 'saved'; state.error = ''; announce();
  }
  function flush() {
    if (running) return pending;
    running = true;
    pending = drain().finally(() => { running = false; });
    return pending;
  }
  return {
    state,
    edit(patch) {
      state.value = validate({ ...state.value, ...patch });
      revision++;
      announce();
      return flush();
    },
    retry: flush,
  };
}
