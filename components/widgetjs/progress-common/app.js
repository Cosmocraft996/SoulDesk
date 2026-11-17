// 根据 WidgetJS 时间/波浪进度源码移植，GPL v2；固定来源见各包 SOURCE.txt。
import { connectHost } from './host.js';
import { accents, labels, remainingPercent, validateSettings } from './periods.js';

const $ = id => document.getElementById(id);
const kind = document.body.dataset.widget;
const form = $('settings-form');
const field = name => form.elements.namedItem(name);
let host, settings, tick, frame, observer;
let saving = Promise.resolve();
let wave = { width: 0, height: 0, surface: 0, last: 0, phase: 0 };
const bars = [];
const media = window.matchMedia('(prefers-reduced-motion: reduce)');
function color() { return settings.accent || accents[settings.period]; }
function render() {
  $('component').style.borderRadius = settings.radius + 'px';
  const now = new Date();
  if (kind === 'time') {
    $('title').textContent = settings.title;
    $('component').style.backgroundColor = settings.background;
    $('component').style.color = settings.foreground;
    bars.forEach(({ fill, value }, period) => {
      const remaining = remainingPercent(now, period);
      fill.style.width = remaining + '%';
      value.textContent = String(remaining).padStart(2, '0') + '%';
    });
  } else {
    $('title').textContent = labels[settings.period] ?? settings.eventName;
    $('percentage').textContent = remainingPercent(now, settings.period, settings.startDate, settings.endDate) + '%';
    $('component').style.backgroundColor = color() + '66';
  }
}
function draw(timestamp = 0) {
  if (timestamp - wave.last >= 1000 / 30 || !timestamp) {
    wave.last = timestamp;
    const canvas = $('wave'), ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('波浪绘图初始化失败');
    const target = wave.height * (100 - remainingPercent(new Date(), settings.period, settings.startDate, settings.endDate)) / 100;
    wave.surface += (target - wave.surface) * .12;
    if (media.matches) wave.surface = target;
    ctx.clearRect(0, 0, wave.width, wave.height);
    if (!media.matches) wave.phase += .035;
    // 沿用原组件的两层贝塞尔波浪；进度越大，水位越低，表示剩余量。
    for (let layer = 1; layer >= 0; layer--) {
      const delta = Math.sin(wave.phase + layer * 1.745) * Math.min(20, wave.height * .06);
      ctx.fillStyle = color() + (layer ? 'cc' : '99');
      ctx.beginPath(); ctx.moveTo(0, wave.surface);
      ctx.bezierCurveTo(wave.width / 4, wave.surface - delta, wave.width * .75, wave.surface + delta, wave.width, wave.surface);
      ctx.lineTo(wave.width, wave.height); ctx.lineTo(0, wave.height); ctx.closePath(); ctx.fill();
    }
  }
  frame = requestAnimationFrame(draw);
}
function customFields() {
  if (kind !== 'wave') return;
  const custom = field('period').value === '4';
  $('custom-fields').hidden = !custom;
  for (const key of ['eventName', 'startDate', 'endDate']) field(key).required = custom;
  field('accent').disabled = $('automatic-color').checked;
}
function localInput(value) {
  if (!value) return '';
  const d = new Date(value);
  if (!Number.isFinite(d.getTime())) return '';
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
}
function openSettings() {
  for (const key of Object.keys(settings)) {
    const input = field(key);
    if (input) input.value = key.endsWith('Date') ? localInput(settings[key]) : String(settings[key]);
  }
  if (kind === 'wave') {
    $('automatic-color').checked = !settings.accent;
    field('accent').value = settings.accent || accents[settings.period];
    customFields();
  }
  $('save-status').textContent = '';
  $('settings').showModal();
}
function close() {
  clearInterval(tick); cancelAnimationFrame(frame); observer?.disconnect();
}
try {
  host = await connectHost();
  settings = validateSettings(kind, await host.request('settings.get'));
  if (kind === 'time') {
    labels.forEach((label, period) => {
      const row = document.createElement('div'); row.className = 'period';
      const text = document.createElement('span'); text.className = 'period-label'; text.textContent = label;
      const track = document.createElement('div'); track.className = 'period-track';
      const fill = document.createElement('div'); fill.className = 'period-fill'; fill.style.backgroundColor = ['#ff8e55', '#ff5594', '#bb55ff', '#6055ff'][period];
      const value = document.createElement('output'); value.className = 'period-value';
      track.append(fill); row.append(text, track, value); $('periods').append(row); bars.push({ fill, value });
    });
  } else {
    observer = new ResizeObserver(() => {
      const canvas = $('wave'), rect = $('component').getBoundingClientRect();
      const ratio = Math.min(window.devicePixelRatio || 1, 3);
      wave.width = rect.width; wave.height = rect.height;
      canvas.width = Math.round(rect.width * ratio); canvas.height = Math.round(rect.height * ratio);
      canvas.getContext('2d').setTransform(ratio, 0, 0, ratio, 0, 0);
    });
    observer.observe($('component'));
    draw();
    field('period').addEventListener('change', customFields);
    $('automatic-color').addEventListener('change', customFields);
  }
  render();
  tick = setInterval(render, 1000);
  $('configure').addEventListener('click', openSettings);
  $('cancel').addEventListener('click', () => $('settings').close());
  async function saveSettings(event) {
    event.preventDefault();
    if ($('save').disabled) return;
    $('save').disabled = true;
    try {
      const next = Object.fromEntries(Object.keys(settings).map(key => [key, field(key)?.value ?? settings[key]]));
      next.radius = Number(next.radius);
      if (kind === 'wave') {
        next.period = Number(next.period);
        next.accent = $('automatic-color').checked ? '' : next.accent;
        for (const key of ['startDate', 'endDate']) if (next[key]) {
          // 在转为UTC前验证日历日期，避免浏览器静默归一化。
          validateSettings('wave', { ...next, period: 4, eventName: next.eventName || '事项', startDate: next.startDate, endDate: next.endDate });
          next[key] = new Date(next[key]).toISOString();
        }
      }
      validateSettings(kind, next);
      const saved = await host.request('settings.update', next);
      settings = validateSettings(kind, saved);
      render(); $('settings').close(); $('status').className = ''; $('status').textContent = '';
    } catch (error) { $('save-status').textContent = '保存失败：' + error.message; }
    finally { $('save').disabled = false; }
  }
  form.addEventListener('submit', event => { saving = saveSettings(event); });
  window.addEventListener('pagehide', close, { once: true });
  await host.ready();
  // 仅隔离验收入口提供此参数；普通使用及隐藏试加载不自动修改任何配置。
  if (!host.preview && new URLSearchParams(location.search).get('verify') === 'widgetjs') {
    const previous = await host.request('storage.get');
    const before = previous.verification;
    let invalidSaveRejected = before?.invalidSaveRejected === true;
    if (!before?.uiVerified) {
      openSettings();
      const savedBefore = JSON.stringify(await host.request('settings.get'));
      if (kind === 'wave') {
        field('period').value = '4'; customFields();
        field('eventName').value = '验证事项';
        field('startDate').value = '2030-01-02T00:00';
        field('endDate').value = '2030-01-01T00:00';
      } else { field('title').value = ' '; }
      form.requestSubmit(); await saving;
      invalidSaveRejected = JSON.stringify(await host.request('settings.get')) === savedBefore && $('save-status').textContent.startsWith('保存失败');
      if (!invalidSaveRejected) throw new Error('无效配置没有被拒绝');
      if (kind === 'wave') {
        field('eventName').value = '事项-' + host.instanceId.slice(0, 8);
        field('startDate').value = '2026-01-01T00:00';
        field('endDate').value = '2027-01-01T00:00';
      } else { field('title').value = '时间-' + host.instanceId.slice(0, 8); }
      field('radius').value = '12';
      form.requestSubmit(); await saving;
      if ($('settings').open) throw new Error('界面配置保存未完成：' + $('save-status').textContent);
    }
    const saved = await host.request('settings.get');
    await host.request('storage.set', { ...previous, verification: {
      uiVerified: true, invalidSaveRejected, runtimeLoads: (before?.runtimeLoads ?? 0) + 1,
      stylesLoaded: getComputedStyle($('component')).display === 'flex' && getComputedStyle($('component')).borderRadius === '12px',
      savedLabel: kind === 'time' ? saved.title : saved.eventName,
      origin: location.origin, renderedText: kind === 'time' ? $('periods').innerText : $('percentage').textContent,
      canvasWidth: kind === 'wave' ? $('wave').width : null,
    } });
  }
} catch (error) {
  close(); $('status').className = 'error'; $('status').textContent = '组件加载失败：' + error.message;
  throw error;
}
