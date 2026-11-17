// 金额和汇率只作为十进制字符串处理，显示时最多保留八位小数。
function decimal(value, rate = false) {
  if (typeof value !== 'string' || value.length > 83 || !/^\d{1,64}(\.\d{1,18})?$/.test(value)) throw new Error(rate ? '汇率需要有效的正十进制数' : '金额最多64位整数、18位小数，不支持负数或科学计数法');
  const [whole, fraction = ''] = value.split('.');
  const digits = BigInt(whole + fraction);
  if (rate && digits === 0n) throw new Error('汇率必须大于0');
  return { digits, scale: fraction.length };
}
export function convertAmount(amount, rate, divide = false) {
  const a = decimal(amount), r = decimal(rate, true), scale = 8;
  const numerator = divide ? a.digits * 10n ** BigInt(r.scale + scale) : a.digits * r.digits * 10n ** BigInt(scale);
  const denominator = divide ? r.digits * 10n ** BigInt(a.scale) : 10n ** BigInt(a.scale + r.scale);
  const rounded = (numerator * 2n + denominator) / (2n * denominator);
  const text = rounded.toString().padStart(scale + 1, '0');
  return (text.slice(0, -scale) + '.' + text.slice(-scale)).replace(/\.?0+$/, '') || '0';
}
export function readRate(response) {
  if (String(response?.ResultCode) !== '0') throw new Error('汇率源返回失败：' + (response?.ResultMsg || response?.ResultCode || '数据缺失'));
  const value = response.Result?.revCode?.[0];
  decimal(value?.price, true);
  if (!Array.isArray(value.list) || !value.list.every(item => item && typeof item.text === 'string' && typeof item.value === 'string')) throw new Error('汇率变动数据格式无效');
  return value;
}
export function createRates(load, changed = () => {}) {
  const state = { pair: null, rate: null, error: '', loading: false, fetchedAt: 0 };
  let revision = 0;
  return {
    state,
    async refresh(from, to) {
      const version = ++revision;
      if (!state.pair || state.pair[0] !== from || state.pair[1] !== to) { state.rate = null; state.fetchedAt = 0; }
      state.pair = [from, to]; state.loading = true; state.error = ''; changed(state);
      try {
        const result = await load([from, to]);
        if (revision !== version) return;
        state.rate = readRate(result.body || result); state.fetchedAt = result.fetchedAt || Date.now();
      } catch (error) { if (revision === version) state.error = String(error.message || error); }
      finally { if (revision === version) { state.loading = false; changed(state); } }
    },
  };
}
