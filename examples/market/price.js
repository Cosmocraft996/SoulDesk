// 仅用于保存前提示；真正触发由 Rust 精确十进制状态机决定。
export function comparePrice(a, b) {
  const parse = value => {
    if (typeof value !== 'string' || value.length > 64 || !/^\d+(\.\d+)?$/.test(value)) throw new Error('请输入正的十进制价格');
    const [integer, fraction = ''] = value.split('.');
    const digits = BigInt(integer + fraction);
    if (digits <= 0n) throw new Error('价格须大于0');
    return { digits, scale: fraction.length };
  };
  const x = parse(a), y = parse(b), scale = Math.max(x.scale, y.scale);
  const left = x.digits * 10n ** BigInt(scale - x.scale), right = y.digits * 10n ** BigInt(scale - y.scale);
  return left === right ? 0 : left > right ? 1 : -1;
}
export function alreadyMet(price, threshold, direction) { const value = comparePrice(price, threshold); return direction === 'up' ? value >= 0 : value <= 0; }
