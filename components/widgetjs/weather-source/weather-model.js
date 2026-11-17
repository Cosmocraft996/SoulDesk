export function temperature(value, unit = 'C') {
  if (typeof value !== 'string' || !/^-?\d{1,3}(?:\.\d{1,3})?$/.test(value)) return null;
  return unit === 'F' ? String(Math.round((Number(value) * 9 / 5 + 32) * 10) / 10) : value;
}
export function observationIsOld(value, time = Date.now()) {
  const observed = Date.parse(value?.obsTime);
  return !Number.isFinite(observed) || time - observed > 2 * 3600000 || observed > time + 300000;
}
function calendarDay(timeZone, time) {
  const parts=Object.fromEntries(new Intl.DateTimeFormat('en',{timeZone,year:'numeric',month:'2-digit',day:'2-digit'}).formatToParts(new Date(time)).map(part=>[part.type,part.value]));
  return `${parts.year}-${parts.month}-${parts.day}`;
}
export function periodIsPast(date, timeZone='Asia/Shanghai', time=Date.now()) {
  return typeof date!=='string'||!/^\d{4}-\d{2}-\d{2}$/.test(date)||date<calendarDay(timeZone,time);
}
export function forecastDayText(date, timeZone='Asia/Shanghai', time=Date.now()) {
  if(typeof date!=='string'||!/^\d{4}-\d{2}-\d{2}$/.test(date))return '日期未提供';
  const today=calendarDay(timeZone,time),days=(Date.parse(date+'T00:00:00Z')-Date.parse(today+'T00:00:00Z'))/86400000;
  if(days<0)return date+'（已过期）';
  return ['今天','明天','后天'][days]||date;
}
export function createWeatherModel(call, options, state = {
  config: { configured: false, enabled: false, apiHost: '' }, now: null, daily: null, indices: [], errors: {}, loading: false,
}) {
  let revision = 0, disposed = false;
  const active = () => !disposed && !options.preview && state.config.configured && state.config.enabled;
  return { state,
    async readStatus() {
      if (disposed || options.preview) return;
      const result = await call('weather.status');
      if (!disposed) state.config = result;
    },
    async lookup(query) {
      if (!active() || !query.trim()) return [];
      const result = await call('weather.get', {kind:'lookup',query:query.trim()});
      if (disposed) return [];
      if (!Array.isArray(result.data?.location)) throw Error('城市列表无效');
      return result.data.location;
    },
    async refresh() {
      if (!active()) return;
      const request = ++revision, city = options.getSettings().cityId;
      state.loading = true;
      const kinds = options.large ? ['now','daily','indices'] : ['now'];
      const results = await Promise.allSettled(kinds.map(kind => call('weather.get', {kind,location:city})));
      if (disposed || request !== revision || city !== options.getSettings().cityId) return;
      for (let index = 0; index < kinds.length; index++) {
        const kind = kinds[index], result = results[index];
        if (result.status === 'rejected') { state.errors[kind] = String(result.reason?.message || result.reason); continue; }
        const data = result.value.data;
        if (kind === 'now' && (!data?.now || temperature(data.now.temp) == null || !Number.isFinite(Date.parse(data.now.obsTime)))) { state.errors.now = '实时天气缺少有效温度或观测时间'; continue; }
        if (kind !== 'now' && !Array.isArray(data?.daily)) { state.errors[kind] = '预报或指数格式无效'; continue; }
        state[kind] = kind === 'now' ? data.now : data.daily;
        delete state.errors[kind];
      }
      state.loading = false;
    },
    reset() { revision++; state.now = null; state.daily = null; state.indices = []; state.errors = {}; state.loading = false; },
    dispose() { disposed = true; revision++; },
  };
}
