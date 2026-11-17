export function validateProxy(value) {
  if (!value || !['inherit','direct','http','socks5'].includes(value.proxyMode)) throw new Error('代理模式无效');
  if (['http','socks5'].includes(value.proxyMode) && (typeof value.proxyHost!=='string' || !value.proxyHost || value.proxyHost.length>253 || /[\/\\@:\s]/.test(value.proxyHost) || !Number.isInteger(value.proxyPort) || value.proxyPort<1 || value.proxyPort>65535)) throw new Error('请输入有效代理主机和1～65535端口，不接收代理凭证');
  return value;
}
