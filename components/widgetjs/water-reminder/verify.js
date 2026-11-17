// 显式隔离验收：通过正式接口检查杯数、表单和画布，不伪造水量或系统时间。
const $ = id => document.getElementById(id);
const sameSettings = (a, b) => Object.keys(a).length === Object.keys(b).length && Object.entries(b).every(([key, value]) => a[key] === value);
async function until(test) {
  for (let n = 0; n < 240; n++) {
    if (await test()) return;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw Error('喝水提醒验收等待超时');
}
async function canvasDrawn() {
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const canvas = $('canvas'), pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
  return { width: canvas.width, height: canvas.height, nonEmpty: pixels.some((v, i) => i % 4 === 3 && v > 0) };
}
export async function verifyWater({ host, model }) {
  if (host.preview || new URLSearchParams(location.search).get('verify') !== 'widgetjs-water') return;
  const stored = await host.request('storage.get'), prior = stored.verification || {};
  async function record(value) { await host.request('storage.set', { ...stored, verification: value }); }
  async function click(id) { $(id).click(); await until(() => !model.state.busy); }
  async function save(value) {
    $('settings').click();
    for (const [key, v] of Object.entries(value)) if (key === 'enableReminder') $(key).checked = v; else $(key).value = String(v);
    $('form').dispatchEvent(new Event('submit', { cancelable: true }));
    await until(() => !$('save').disabled);
  }
  try {
    await model.refresh();
    if (prior.uiVerified) {
      const snapshot = model.state.snapshot, settings = await host.request('settings.get'), canvas = await canvasDrawn();
      const configurationRetained = sameSettings(settings, prior.testSettings), businessStateVerified = snapshot?.count === prior.expectedCount && snapshot?.day === prior.day;
      await record({ ...prior, runtimeLoads: prior.runtimeLoads + 1, configurationRetained, businessStateVerified, canvas, uiVerified: configurationRetained && businessStateVerified && canvas.nonEmpty });
      return;
    }
    const registered = model.state.snapshot?.count === 0 && model.state.snapshot?.nextReminderAt > model.state.snapshot?.sampledAt;
    await click('add'); const addWorks = model.state.snapshot?.count === 1;
    await click('reset'); const resetWorks = model.state.snapshot?.count === 0;
    const requestId = crypto.randomUUID(), first = await host.request('water.change', { requestId, operation: 'add' }), repeated = await host.request('water.change', { requestId, operation: 'add' });
    const idempotentAdd = first.count === 1 && repeated.count === 1;
    let conflictingRequestRejected = false;
    try { await host.request('water.change', { requestId, operation: 'reset' }); } catch { conflictingRequestRejected = true; }
    await model.refresh(); await click('add');
    const color = '#' + host.instanceId.replaceAll('-', '').slice(0, 6);
    await save({ enableReminder: true, interval: 5, targetCup: 2, background: color, color: '#ffffff', primary: '#2596ff' });
    const targetReached = model.state.snapshot?.count === 2 && model.state.snapshot?.nextReminderAt === null && $('status').textContent === '今日目标已完成';
    const before = await host.request('settings.get'); await save({ interval: 4 });
    const invalidSaveRejected = JSON.stringify(await host.request('settings.get')) === JSON.stringify(before) && $('save-error').textContent.includes('设置未保存');
    const suffix = parseInt(host.instanceId.slice(-2), 16), testSettings = { enableReminder: false, interval: 5 + suffix % 96, targetCup: 8 + suffix % 12, background: color, color: '#ffffff', primary: '#2596ff' };
    await save(testSettings);
    for (let n = 0; n < suffix % 3; n++) await click('add');
    const expectedCount = 2 + suffix % 3, snapshot = model.state.snapshot, canvas = await canvasDrawn(), actualSettings = await host.request('settings.get');
    const configurationSaved = sameSettings(actualSettings, testSettings), businessStateVerified = snapshot?.count === expectedCount && snapshot?.nextReminderAt === null && snapshot?.enableReminder === false && $('count').textContent === expectedCount + '/' + testSettings.targetCup;
    const stylesLoaded = getComputedStyle($('water')).display === 'flex' && getComputedStyle($('canvas')).display === 'block', disabledStatusShown = $('status').textContent === '提醒已关闭';
    await record({ runtimeLoads: 1, registered, addWorks, resetWorks, idempotentAdd, conflictingRequestRejected, targetReached, invalidSaveRejected, configurationSaved, configurationRetained: configurationSaved, businessStateVerified, stylesLoaded, disabledStatusShown, canvas, expectedCount, day: snapshot.day, testSettings: actualSettings, actualMouseVerified: false, actualNotificationVerified: false, scope: 'production-water-gateway-and-WebView; programmatic-buttons-settings-and-canvas', uiVerified: registered && addWorks && resetWorks && idempotentAdd && conflictingRequestRejected && targetReached && invalidSaveRejected && configurationSaved && businessStateVerified && stylesLoaded && disabledStatusShown && canvas.nonEmpty });
  } catch (error) { await record({ ...prior, phase: 'failed', failure: String(error?.message || error), uiVerified: false }); }
}
