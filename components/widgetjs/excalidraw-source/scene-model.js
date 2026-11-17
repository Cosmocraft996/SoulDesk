function object(value) { return value && typeof value === 'object' && !Array.isArray(value); }
function validateScene(scene) {
  if (scene === null) return;
  if (!object(scene) || scene.type !== 'excalidraw' || scene.version !== 2 || !Array.isArray(scene.elements) || !object(scene.appState) || !object(scene.files)) throw new Error('画板存储格式无效，请保留原数据后检查');
  if (!scene.elements.every(object) || !Object.values(scene.files).every(file => object(file) && typeof file.dataURL === 'string' && /^data:image\/[a-z0-9+.-]+[;,]/i.test(file.dataURL))) throw new Error('画板图形或图片存储无效');
}
export function restoreScene(stored) {
  if (!object(stored)) throw new Error('画板存储不是有效对象');
  const value = { scene:null, libraryItems:[], ...structuredClone(stored) };
  validateScene(value.scene);
  if (!Array.isArray(value.libraryItems) || !value.libraryItems.every(object)) throw new Error('画板图形库存储无效');
  return value;
}
export function createSceneStore(stored, persist, changed = (_state) => {}, limit = 32 * 1024 * 1024) {
  let value = restoreScene(stored), signature = JSON.stringify(value), revision = 0, saved = 0, blocked = '';
  let running = false, pending;
  const state = { status:'saved', error:'' };
  function announce() { changed(state); }
  function validate() {
    validateScene(value.scene);
    if (new TextEncoder().encode(signature).length > limit) throw new Error('画板超过实例存储上限，最新编辑未自动保存；请缩小图片或导出文件');
  }
  validate();
  async function drain() {
    while (saved !== revision && !blocked) {
      const writing = revision, snapshot = value;
      state.status = 'saving'; state.error = ''; announce();
      try { await persist(snapshot); }
      catch (error) { state.status = 'error'; state.error = blocked || String(error?.message || error); announce(); return; }
      saved = writing;
    }
    state.status = blocked ? 'error' : 'saved'; state.error = blocked; announce();
  }
  function flush() {
    if (blocked) return Promise.resolve();
    if (running) return pending;
    running = true; pending = drain().finally(() => { running = false; }); return pending;
  }
  function update(patch) {
    const next = { ...value, ...structuredClone(patch) }, nextSignature = JSON.stringify(next);
    if (nextSignature === signature) return pending || Promise.resolve();
    value = next; signature = nextSignature; revision++;
    try { validate(); blocked = ''; }
    catch (error) { blocked = error.message; state.status = 'error'; state.error = blocked; announce(); return pending || Promise.resolve(); }
    return flush();
  }
  return { state, updateScene: scene => update({scene}), updateLibrary: libraryItems => update({libraryItems}), retry:flush };
}
