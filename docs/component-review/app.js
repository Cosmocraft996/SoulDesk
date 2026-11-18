/* 本机草稿即时缓存；服务端确认保存后才显示“已保存”。 */
(() => {
  const $ = selector => document.querySelector(selector);
  const decisions = { keep: '保留', trim: '裁剪', undecided: '待定' };
  const results = { untested: '待验证', passed: '通过', partial: '部分通过', failed: '失败' };
  const cacheKey = 'souldesk-component-review-v1';
  let catalog, state, category = '全部组件', dirty = false, sequence = 0, saving = null, timer, toastTimer, conflict = false;
  const escape = value => String(value).replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
  async function api(path, options) {
    const response = await fetch(path, { cache: 'no-store', ...options });
    const value = await response.json();
    if (!response.ok) throw Object.assign(Error(value.error || '请求失败'), { status: response.status });
    return value;
  }
  function status(text, kind = '') { $('#save-status').textContent = text; $('#save-dot').className = 'save-dot ' + kind; $('#retry-save').hidden = kind !== 'error' || conflict; }
  function toast(text, error = false) { clearTimeout(toastTimer); $('#toast').textContent = text; $('#toast').className = 'toast' + (error ? ' error' : ''); $('#toast').hidden = false; toastTimer = setTimeout(() => { $('#toast').hidden = true; }, 10000); }
  function cache() {
    try { localStorage.setItem(cacheKey, JSON.stringify({ ...state, pending: dirty })); }
    catch { status('浏览器备份不可用，正在保存到本机项目', 'error'); }
  }
  function changed() { sequence++; dirty = true; cache(); status('有修改，等待保存…'); clearTimeout(timer); timer = setTimeout(() => { save().catch(() => {}); }, 650); updateStats(); }
  async function save() {
    clearTimeout(timer);
    if (saving) { await saving; if (dirty) return save(); return; }
    if (!dirty) return;
    if (conflict) throw Error('草稿有冲突，请先导出备份再刷新核对');
    saving = (async () => {
      while (dirty) {
        const sentSequence = sequence, payload = JSON.stringify(state);
        status('正在保存草稿…');
        const saved = await api('/api/state', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: payload });
        state.revision = saved.revision; state.updatedAt = saved.updatedAt;
        dirty = sequence !== sentSequence; cache();
      }
      status('草稿已保存到本机 · ' + new Date(state.updatedAt).toLocaleTimeString('zh-CN', { hour12: false }), 'saved');
    })();
    try { await saving; }
    catch (error) { conflict = error.status === 409; status('保存失败：' + error.message, 'error'); throw error; }
    finally { saving = null; }
  }
  function updateStats() {
    const entries = Object.values(state.entries), counts = Object.fromEntries(Object.keys(decisions).map(key => [key, entries.filter(entry => entry.decision === key).length]));
    const selected = entries.filter(entry => entry.decision).length;
    for (const key of Object.keys(counts)) $('#' + key + '-count').textContent = counts[key];
    $('#selected-count').textContent = selected; $('#progress-bar').style.width = selected / catalog.items.length * 100 + '%';
    $('#verified-count').textContent = entries.filter(entry => entry.result !== 'untested').length;
  }
  function renderCategories() {
    const names = ['全部组件', ...new Set(catalog.items.map(item => item.category))];
    $('#categories').innerHTML = names.map(name => `<button type="button" class="${name === category ? 'active' : ''}" data-category="${escape(name)}" aria-pressed="${name === category}">${escape(name)}<span>${name === '全部组件' ? catalog.items.length : catalog.items.filter(item => item.category === name).length}</span></button>`).join('');
  }
  function render() {
    const query = $('#search').value.trim().toLowerCase(), decision = $('#decision-filter').value, result = $('#result-filter').value;
    const pool = catalog.items;
    const visible = pool.filter(item => {
      const entry = state.entries[item.id];
      return (category === '全部组件' || item.category === category) && (decision === 'all' || (decision === 'unselected' ? !entry.decision : entry.decision === decision)) &&
        (result === 'all' || entry.result === result) && (!query || [item.name, item.slug, item.summary, item.originalTitle, item.originalDescription].join(' ').toLowerCase().includes(query));
    });
    $('#visible-count').textContent = `${category} · 显示 ${visible.length} / ${pool.length} 项`;
    $('#empty').hidden = visible.length > 0;
    $('#records').innerHTML = visible.map(item => {
      const entry = state.entries[item.id];
      return `<article class="record" data-id="${escape(item.id)}" data-result="${entry.result}">
        <div class="record-main"><span class="record-number">${String(item.number).padStart(2, '0')}</span>
          <div class="record-info"><h2>${escape(item.name)}</h2><span class="category-tag">${escape(item.category)}</span>
            <div class="identity">${escape(item.slug)} · v${escape(item.version)}${item.cloudDeferred ? '<span class="cloud-tag">原云功能后置</span>' : ''}</div>
            <p class="description">${escape(item.summary)}</p><div class="record-links">
              <a href="${escape(item.projectUrl)}" target="_blank" rel="noopener noreferrer">原项目 ↗</a>
              ${item.pinnedUrl !== item.projectUrl ? `<a href="${escape(item.pinnedUrl)}" target="_blank" rel="noopener noreferrer">固定版本 ↗</a>` : ''}
              <a href="/api/source?id=${encodeURIComponent(item.id)}" target="_blank" rel="noopener noreferrer">移植来源说明 ↗</a>
            </div></div>
          <div class="record-controls"><div class="decision-group" role="group" aria-label="${escape(item.name)}的筛选意向">
            ${Object.entries(decisions).map(([value, label]) => `<label><input type="radio" name="decision-${item.number}" data-field="decision" value="${value}" ${entry.decision === value ? 'checked' : ''}><span>${label}</span></label>`).join('')}
          </div><label class="result-control">验证结果<select data-field="result" aria-label="${escape(item.name)}的验证结果">${Object.entries(results).map(([value, label]) => `<option value="${value}" ${entry.result === value ? 'selected' : ''}>${label}</option>`).join('')}</select></label></div>
        </div><details ${entry.notes || entry.result === 'failed' || entry.result === 'partial' ? 'open' : ''}><summary>${entry.notes ? '已记录问题与备注' : '记录问题与备注'}</summary><div class="note-fields">
          <label>实测组件版本<input data-field="testedVersion" maxlength="120" value="${escape(entry.testedVersion)}" aria-label="${escape(item.name)}的实测版本"></label>
          <label>验证过程、问题或裁剪原因<textarea data-field="notes" maxlength="8000" placeholder="例如：设置保存正常；移到桌面后文字被截断。">${escape(entry.notes)}</textarea></label>
        </div></details></article>`;
    }).join('');
  }
  function resetFilters() { category = '全部组件'; $('#search').value = ''; $('#decision-filter').value = 'all'; $('#result-filter').value = 'all'; renderCategories(); render(); }
  function exportBackup() {
    const report = { schemaVersion: 1, exportedAt: new Date().toISOString(), build: catalog.build, catalogId: catalog.id, items: catalog.items.map(item => ({ ...item, ...state.entries[item.id] })) };
    const url = URL.createObjectURL(new Blob([JSON.stringify(report, null, 2)], { type: 'application/json' }));
    const anchor = document.createElement('a'); anchor.href = url; anchor.download = `SoulDesk-组件筛选-${new Date().toISOString().slice(0, 10)}.json`; anchor.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  async function boot() {
    try {
      [catalog, state] = await Promise.all([api('/api/catalog'), api('/api/state')]);
      let cached;
      try {
        const stored = JSON.parse(localStorage.getItem(cacheKey));
        if (stored?.entries) {
          cached = { schemaVersion: stored.schemaVersion, catalogId: stored.catalogId, revision: stored.revision,
            updatedAt: stored.updatedAt, pending: stored.pending,
            entries: Object.fromEntries(catalog.items.filter(item => stored.entries[item.id]).map(item => [item.id, stored.entries[item.id]])) };
          localStorage.setItem(cacheKey, JSON.stringify(cached));
        }
      } catch { /* 损坏的浏览器备份不替换本机草稿。 */ }
      if (cached?.pending && cached.catalogId === catalog.id && cached.revision === state.revision) {
        state.entries = Object.fromEntries(catalog.items.map(item => [item.id, cached.entries[item.id] || state.entries[item.id]]));
        dirty = true; toast('已恢复浏览器中尚未保存的输入。');
      } else if (cached?.pending) {
        // 有冲突的浏览器输入仍可导出，不能被读取旧草稿的动作静默覆盖。
        $('#fatal').hidden = false; $('#fatal').textContent = '检测到另一份未保存的浏览器草稿，已保留在浏览器中。请先下载该草稿备份，再继续填写。';
        const button = document.createElement('button'); button.className = 'secondary'; button.textContent = '下载未保存草稿';
        button.onclick = () => { const url = URL.createObjectURL(new Blob([JSON.stringify(cached, null, 2)], { type: 'application/json' })); const link = document.createElement('a'); link.href = url; link.download = 'SoulDesk-未保存草稿.json'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000); button.disabled = true; button.textContent = '已发起备份下载，可继续填写'; $('#records').inert = false; $('#submit').disabled = false; conflict = false; status('已恢复本机草稿，可继续填写', 'saved'); };
        $('#fatal').append(document.createElement('br'), button); $('#records').inert = true; conflict = true;
      }
      $('#build').textContent = `${catalog.build.version} / ${catalog.build.label}`;
      $('#catalog-summary').textContent = `源码 ${catalog.sourceVersion} · 当前 ${catalog.items.length} 项`;
      $('#total-count').textContent = '/ ' + catalog.items.length;
      renderCategories(); render(); updateStats();
      $('#export').disabled = false; $('#submit').disabled = conflict;
      if (dirty) { cache(); save().catch(() => {}); }
      else if (!conflict) status(state.updatedAt ? '草稿已从本机恢复' : '尚未填写 · 修改后自动保存', 'saved');
      $('#search').addEventListener('input', render); $('#decision-filter').addEventListener('change', render); $('#result-filter').addEventListener('change', render);
      $('#categories').addEventListener('click', event => { const button = event.target.closest('[data-category]'); if (!button) return; category = button.dataset.category; renderCategories(); render(); });
      document.querySelectorAll('[data-decision-filter]').forEach(button => button.addEventListener('click', () => { $('#decision-filter').value = button.dataset.decisionFilter; render(); }));
      $('#reset-filters').addEventListener('click', resetFilters); $('#retry-save').addEventListener('click', () => save().catch(() => {}));
      $('#records').addEventListener('input', event => {
        const field = event.target.dataset.field, row = event.target.closest('[data-id]');
        if (!field || !row) return;
        state.entries[row.dataset.id][field] = event.target.value; row.dataset.result = state.entries[row.dataset.id].result;
        if (field === 'result' && ['partial', 'failed'].includes(event.target.value)) row.querySelector('details').open = true;
        changed();
      });
      $('#export').addEventListener('click', exportBackup);
      $('#submit').addEventListener('click', () => {
        const entries = Object.values(state.entries), unselected = entries.filter(entry => !entry.decision).length, untested = entries.filter(entry => entry.result === 'untested').length;
        $('#submission-summary').innerHTML = Object.entries(decisions).map(([value, label]) => `<div>${label}<strong>${entries.filter(entry => entry.decision === value).length}</strong></div>`).join('');
        $('#submission-note').textContent = unselected || untested ? `还有 ${unselected} 项未筛选、${untested} 项待验证。可以提交当前进度，之后继续填写。` : `${catalog.items.length} 项已完成筛选与验证记录。可以提交本次结果。`;
        $('#submit-dialog').showModal();
      });
      for (const id of ['cancel-submit', 'back-submit']) $('#' + id).addEventListener('click', () => $('#submit-dialog').close());
      $('#confirm-submit').addEventListener('click', async () => {
        const button = $('#confirm-submit'); button.disabled = true; button.textContent = '正在提交…';
        try {
          await save();
          const receipt = await api('/api/submit', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(state) });
          state.revision = receipt.revision; state.updatedAt = receipt.submittedAt; dirty = false; cache(); $('#submit-dialog').close();
          status('本次结果已提交到本机，后续修改会继续保存草稿', 'saved');
          toast('已提交到项目目录。\n告诉我“已提交”，即可继续汇总。\n' + receipt.files[1]);
        } catch (error) { toast('提交失败：' + error.message, true); }
        finally { button.disabled = false; button.textContent = '提交到本机'; }
      });
      window.addEventListener('beforeunload', event => { if (dirty) { cache(); event.preventDefault(); event.returnValue = ''; } });
    } catch (error) { $('#fatal').hidden = false; $('#fatal').textContent = '加载失败：' + error.message; status('加载未完成', 'error'); }
  }
  boot();
})();
