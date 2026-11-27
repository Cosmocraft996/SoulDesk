import { useEffect, useRef, useState } from 'react';
import { invoke } from './host';

const markets = {
  usd: '普通 U 本位合约',
  coin: '币本位合约',
  spot: '现货',
  portfolio: '统一账户的 U 本位',
} as const;
type Market = keyof typeof markets;
type Profile = { market: Market; configured: boolean; trading: boolean; credentialError: boolean };
type Pending = { id: string; label: string; createdAt: number };
type AccountStatus = {
  profiles: Profile[];
  package: { digest: string; version: string; approved: boolean } | null;
  pending: { market: Market; record: Pending }[];
  archive: { market: Market; record: { id: string; label: string; archivedAt: number } }[];
};
const request = (args: object) => invoke<AccountStatus>('binance_account_settings', { args });

export function BinanceAccountSettings() {
  const [market, setMarket] = useState<Market>('usd');
  const [status, setStatus] = useState<AccountStatus>();
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const key = useRef<HTMLInputElement>(null);
  const secret = useRef<HTMLInputElement>(null);
  const profile = status?.profiles.find(item => item.market === market);
  const pending = status?.pending.find(item => item.market === market)?.record;
  const archived = status?.archive.find(item => item.market === market)?.record;
  const installed = status?.package;
  useEffect(() => {
    let mounted = true;
    void request({ action: 'get' }).then(result => {
      if (mounted) setStatus(result);
    }).catch(issue => {
      if (mounted) setError(String(issue));
    });
    return () => { mounted = false; };
  }, []);
  function clearFields() {
    if (key.current) key.current.value = '';
    if (secret.current) secret.current.value = '';
  }
  async function run(action: 'save' | 'test' | 'remove' | 'setTrading' | 'get' | 'approvePackage' | 'archivePending', trading = false) {
    setBusy(true);
    setError('');
    setMessage('');
    try {
      const args = action === 'save'
        ? { action, market, apiKey: key.current?.value.trim(), secret: secret.current?.value.trim(), trading: false }
        : action === 'setTrading' ? { action, market, trading }
        : action === 'approvePackage' ? { action, digest: installed?.digest }
        : action === 'archivePending' ? { action, market, id: pending?.id }
        : action === 'get' ? { action }
        : { action, market };
      setStatus(await request(args));
      if (action === 'save' || action === 'remove') clearFields();
      setMessage({
        get: '', save: '已保存，当前为只读', test: '账户读取通过', remove: '已移除本机凭据',
        setTrading: trading ? '已允许经宿主确认的交易操作' : '已关闭交易权限',
        approvePackage: '已授权当前组件包读取已配置的币安账户',
        archivePending: '已归档，交易锁定已解除。原指令结果仍待确认',
      }[action]);
    } catch (issue) {
      setError(String(issue));
    } finally {
      setBusy(false);
    }
  }
  return <section className="settings-panel" id="binance-account-config" tabIndex={-1}>
    <h2>币安账户监控</h2>
    <p>使用币安 HMAC API 密钥。凭据由 Windows 当前用户加密保存，四类账户独立配置。</p>
    {installed ? <details open={!installed.approved}>
      <summary>组件 {installed.version} · {installed.approved ? '已授权' : '待授权'}</summary>
      <p>核对安装包来源及 SHA-256 后，可授权此版本读取已配置的币安账户。组件包更新后需要重新确认。</p>
      <p><code style={{ overflowWrap: 'anywhere' }}>{installed.digest}</code></p>
      {!installed.approved && <button type="button" disabled={busy} onClick={() => void run('approvePackage')}>授权此组件包</button>}
    </details> : <p>尚未导入币安账户组件包。</p>}
    <div className="network-grid">
      <label>账户类型<select disabled={busy} value={market} onChange={e => {
        clearFields();
        setError('');
        setMessage('');
        setMarket(e.target.value as Market);
      }}>
        {Object.entries(markets).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </select></label>
      <p>{profile?.credentialError ? '本机凭据无法解密' : profile?.configured ? '已配置' : '未配置'}</p>
    </div>
    <div className="actions"><button type="button" disabled={busy} onClick={() => void run('get')}>刷新状态</button></div>
    {pending && <div className="notice">
      <p>有待核对指令。请核对币安订单、成交或合约设置后再归档。</p>
      <p>{pending.label} · <code>{pending.id}</code></p>
      <button type="button" disabled={busy} onClick={() => void run('archivePending')}>归档并解除锁定</button>
    </div>}
    {archived && <details>
      <summary>最近归档的指令 · 结果未确认</summary>
      <p>{new Date(archived.archivedAt).toLocaleString('zh-CN')} · <code>{archived.id}</code></p>
      <p>{archived.label}</p>
    </details>}
    <form onSubmit={event => { event.preventDefault(); void run('save'); }}>
      <div className="network-grid">
        <label>API Key<input ref={key} type="password" required minLength={16} maxLength={256} autoComplete="off" disabled={busy} /></label>
        <label>API 密钥<input ref={secret} type="password" required minLength={16} maxLength={256} autoComplete="off" disabled={busy} /></label>
      </div>
      <div className="actions">
        <button type="submit" disabled={busy}>{profile?.configured ? '更换凭据' : '保存凭据'}</button>
        <button type="button" disabled={busy || !profile?.configured} onClick={() => void run('test')}>测试读取</button>
        <button type="button" disabled={busy || (!profile?.configured && !profile?.credentialError)} onClick={() => void run('remove')}>移除本机凭据</button>
      </div>
    </form>
    <label className="check">
      <input type="checkbox" checked={profile?.trading ?? false} disabled={busy || !profile?.configured}
        onChange={e => void run('setTrading', e.target.checked)} />
      允许组件提交经宿主确认的交易指令
    </label>
    {message && <p role="status">{message}</p>}
    {error && <p role="alert" className="notice error">{error}</p>}
  </section>;
}
