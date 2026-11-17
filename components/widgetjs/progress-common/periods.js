// 根据 WidgetJS 进度组件移植，GPL v2；固定来源及改动见各包 SOURCE.txt。
export const labels = ['今天', '本周', '本月', '今年'];
export const accents = ['#ff5594', '#ff8e55', '#bb55ff', '#6055ff', '#ff8e55'];

export function periodBounds(now, type, startDate = '', endDate = '') {
  if (!(now instanceof Date) || !Number.isFinite(now.getTime())) throw new Error('当前时间无效');
  if (!Number.isInteger(type) || type < 0 || type > 4) throw new Error('进度类型无效');
  let start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  let end = new Date(start);
  switch (type) {
    case 0: end.setDate(end.getDate() + 1); break;
    case 1:
      start.setDate(start.getDate() - (start.getDay() + 6) % 7);
      end = new Date(start); end.setDate(end.getDate() + 7);
      break;
    case 2:
      start = new Date(now.getFullYear(), now.getMonth(), 1);
      end = new Date(now.getFullYear(), now.getMonth() + 1, 1);
      break;
    case 3:
      start = new Date(now.getFullYear(), 0, 1);
      end = new Date(now.getFullYear() + 1, 0, 1);
      break;
    case 4:
      start = parseDate(startDate); end = parseDate(endDate);
      break;
  }
  if (end <= start) throw new Error('结束时间必须晚于开始时间');
  return { start, end };
}

export function parseDate(value) {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?(?:Z|[+-]\d{2}:\d{2})?$/.test(value)) throw new Error('请输入有效的日期与时间');
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) throw new Error('日期与时间无效');
  // 浏览器会把2月30日等日期自动归一化，必须在保存前明确拒绝。
  const [year, month, day, hour, minute, second = 0] = value.slice(0, 19).split(/[-T:]/).map(Number);
  const calendar = new Date(Date.UTC(year, month - 1, day));
  if (month < 1 || month > 12 || day < 1 || calendar.getUTCDate() !== day || hour > 23 || minute > 59 || second > 59) throw new Error('日期与时间无效');
  return date;
}

export function elapsedPercent(now, type, startDate, endDate) {
  const { start, end } = periodBounds(now, type, startDate, endDate);
  return Math.max(0, Math.min(100, Math.round((now - start) / (end - start) * 100)));
}

export function remainingPercent(now, type, startDate, endDate) {
  return 100 - elapsedPercent(now, type, startDate, endDate);
}

export function validateSettings(kind, settings) {
  const color = value => typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value);
  if (!Number.isInteger(settings.radius) || settings.radius < 0 || settings.radius > 40) throw new Error('圆角范围为0至40');
  if (kind === 'time') {
    if (typeof settings.title !== 'string' || !settings.title.trim() || [...settings.title].length > 40) throw new Error('标题需要1至40个字符');
    if (!color(settings.background) || !color(settings.foreground)) throw new Error('颜色需要六位十六进制值');
  } else if (kind === 'wave') {
    if (!Number.isInteger(settings.period) || settings.period < 0 || settings.period > 4) throw new Error('进度类型无效');
    if (settings.accent !== '' && !color(settings.accent)) throw new Error('波浪颜色无效');
    if (typeof settings.eventName !== 'string' || [...settings.eventName].length > 40) throw new Error('事项名称最多40个字符');
    if (settings.period === 4) {
      if (!settings.eventName.trim()) throw new Error('请填写事项名称');
      periodBounds(new Date(), 4, settings.startDate, settings.endDate);
    }
  } else throw new Error('组件类型无效');
  return settings;
}
