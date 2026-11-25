import { useCallback, useEffect, useRef, useState } from 'react';
import appPackage from '../package.json' with { type: 'json' };
import { invoke } from './host';
import { WidgetBrowser, widgetCategory } from './WidgetBrowser';

interface Release {
  version: string;
  minHostVersion: string;
  capabilities: string[];
  releaseNotes: string;
  status: 'published' | 'withdrawn';
  sha256: string;
}

interface Entry {
  id: string;
  name: string;
  summary: string;
  category: string;
  versions: Release[];
}

export interface CatalogResponse {
  catalog: { revision: number; widgets: Entry[] } | null;
  online: boolean;
  error: string | null;
}

interface LocalPackage {
  id: string;
  version: string;
  digest?: string;
  capabilities?: string[];
}

interface Props {
  packages: LocalPackage[];
  instances: { packageId: string }[];
  unavailable: boolean;
  canAdd: boolean;
  onAdd: (id: string) => void;
  onUninstall: (id: string) => void;
  onManage: (id: string) => void;
  onRefresh: () => Promise<void>;
}

const permissionNames: Record<string, string> = {
  'storage.instance': '读取和保存当前实例的设置与存储',
  'links.metadata': '读取公网网页标题和图标',
  'links.verify': '打开临时网页验证窗口',
};

function compareVersion(lhs: string, rhs: string) {
  const left = lhs.split('.').map(BigInt);
  const right = rhs.split('.').map(BigInt);
  for (let index = 0; index < 3; index++) {
    if (left[index] !== right[index]) return left[index] > right[index] ? 1 : -1;
  }
  return 0;
}

function permissionText(capabilities: string[]) {
  return capabilities.map(item => permissionNames[item] ?? item).join('、') || '无需额外权限';
}

interface Confirmation { entry: Entry; release: Release; local?: LocalPackage }
type Catalog = NonNullable<CatalogResponse['catalog']>;
type PendingUpdate = Confirmation & { local: LocalPackage & { digest: string } };

function publishedReleases(entry: Entry) {
  return entry.versions.filter(item => item.status === 'published')
    .sort((a, b) => compareVersion(b.version, a.version));
}

function compatibleRelease(releases: Release[]) {
  return releases.find(item => compareVersion(appPackage.version, item.minHostVersion) >= 0);
}

function addedPermissions({ local, release }: Confirmation) {
  return release.capabilities.filter(permission => !local?.capabilities?.includes(permission));
}

function installRelease({ entry, release, local }: Confirmation) {
  return invoke('install_official', {
    packageId: entry.id, expectedSha256: release.sha256,
    ...(local?.digest ? { expectedLocalDigest: local.digest } : {}),
  });
}

function ConfirmRelease({ target, onClose, onInstalled }: {
  target: Confirmation;
  onClose: () => void;
  onInstalled: () => Promise<void>;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const update = !!target.local;
  const newPermissions = addedPermissions(target);
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    return () => element.close();
  }, []);
  async function install() {
    setBusy(true);
    setError('');
    try {
      await installRelease(target);
      await onInstalled();
      onClose();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }
  return <dialog ref={dialog} className="remove-instance-dialog gallery-confirm" aria-label="官方组件安装确认"
    onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}>
    <form onSubmit={event => { event.preventDefault(); if (!busy) void install(); }}>
      <h2>{update ? '更新' : '安装'}「{target.entry.name}」</h2>
      <p>版本 {target.release.version} · 要求 SoulDesk {target.release.minHostVersion} 或以上</p>
      <p>权限 · {permissionText(target.release.capabilities)}</p>
      {update && <p>新增权限 · {newPermissions.length ? permissionText(newPermissions) : '无'}</p>}
      <p className="gallery-release-notes">{target.release.releaseNotes}</p>
      {error && <p role="alert" className="error">{error}</p>}
      <div className="actions">
        <button type="button" autoFocus disabled={busy} onClick={onClose}>取消</button>
        <button className="primary" disabled={busy}>{busy ? '正在下载和试加载…' : (update ? '确认更新' : '确认安装')}</button>
      </div>
    </form>
  </dialog>;
}

export function UpdateAllWidgets({ packages, onCatalog, onClose, onInstalled }: {
  packages: LocalPackage[];
  onCatalog: (catalog: Catalog) => void;
  onClose: () => void;
  onInstalled: () => Promise<void>;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const started = useRef(false);
  const [check, setCheck] = useState<{ catalog?: Catalog; error?: string }>();
  const [batch, setBatch] = useState<{ targets: PendingUpdate[]; outcomes: (string | null)[]; done: boolean }>();
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    let current = true;
    void invoke<CatalogResponse>('official_catalog').then(result => {
      if (!current) return;
      if (result.catalog) onCatalog(result.catalog);
      if (result.online && result.catalog) setCheck({ catalog: result.catalog });
      else setCheck({ error: result.error || '官方目录不可用，请稍后重试。' });
    }, error => { if (current) setCheck({ error: String(error) }); });
    return () => { current = false; element.close(); };
  }, [onCatalog]);

  const targets = batch?.targets ?? check?.catalog?.widgets.flatMap(entry => {
    const local = packages.find(pkg => pkg.id === entry.id);
    const release = compatibleRelease(publishedReleases(entry));
    if (!local?.digest || !release || compareVersion(local.version, release.version) >= 0) return [];
    return [{ entry, release, local: { ...local, digest: local.digest } }];
  }) ?? [];
  const running = !!batch && !batch.done;
  const failures = batch?.outcomes.filter(error => error !== null).length ?? 0;

  async function update() {
    if (started.current || !targets.length) return;
    started.current = true;
    const outcomes: (string | null)[] = [];
    setBatch({ targets, outcomes: [], done: false });
    for (const target of targets) {
      try {
        await installRelease(target);
        outcomes.push(null);
      } catch (cause) {
        outcomes.push(String(cause));
      }
      setBatch({ targets, outcomes: [...outcomes], done: false });
    }
    await onInstalled();
    setBatch({ targets, outcomes, done: true });
  }

  return <dialog ref={dialog} className="remove-instance-dialog gallery-batch" aria-label="全部更新组件"
    onCancel={event => { event.preventDefault(); if (!running) onClose(); }}>
    <form onSubmit={event => { event.preventDefault(); void update(); }}>
      <h2>全部更新组件</h2>
      <p role="status">
        {!check ? '正在检查官方组件更新…' : check.error ? '无法在线检查组件更新。'
          : batch?.done ? `已更新 ${targets.length - failures} 个，失败 ${failures} 个。`
          : running ? `已处理 ${batch.outcomes.length} / ${targets.length} 个组件。`
          : targets.length ? `有 ${targets.length} 个已安装组件可更新。` : '已安装组件暂无兼容的新版。'}
      </p>
      {check?.error && <p className="error" role="alert">{check.error}</p>}
      {targets.length > 0 && <ul className="gallery-update-list">
        {targets.map((target, index) => {
          const permissions = addedPermissions(target);
          const outcome = batch?.outcomes[index];
          return <li key={target.entry.id}>
            <h3>{target.entry.name}</h3>
            <p>v{target.local.version} → v{target.release.version} · 新增权限 · {permissions.length ? permissionText(permissions) : '无'}</p>
            {outcome === null ? <p>已更新</p>
              : outcome !== undefined ? <p className="error">更新失败：{outcome}</p>
              : running && index === batch.outcomes.length && <p>正在下载和试加载…</p>}
          </li>;
        })}
      </ul>}
      <div className="actions">
        <button type="button" autoFocus disabled={running} onClick={onClose}>{!batch && targets.length ? '取消' : '关闭'}</button>
        {!batch && targets.length > 0 && <button className="primary">确认全部更新</button>}
      </div>
    </form>
  </dialog>;
}

export function OfficialGallery({ packages, instances, unavailable, canAdd, onAdd, onUninstall, onManage, onRefresh }: Props) {
  const [result, setResult] = useState<CatalogResponse>();
  const [loading, setLoading] = useState(true);
  const [confirmation, setConfirmation] = useState<Confirmation>();
  const request = useRef(0);
  const refresh = useCallback(async () => {
    const current = ++request.current;
    setLoading(true);
    try {
      const response = await invoke<CatalogResponse>('official_catalog');
      if (request.current === current) setResult(response);
    } catch (error) {
      if (request.current === current) {
        setResult(previous => ({ catalog: previous?.catalog ?? null, online: false, error: String(error) }));
      }
    } finally {
      if (request.current === current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    void refresh();
    return () => { request.current++; };
  }, [refresh]);

  const entries = result?.catalog?.widgets.map(entry => ({
    ...entry, category: widgetCategory(entry.id, entry.category),
  })) ?? [];

  return <>
    <div className="heading"><p>浏览、安装和更新 SoulDesk 官方组件。</p>
      <button disabled={loading} onClick={() => void refresh()}>{loading ? '正在刷新…' : '刷新目录'}</button>
    </div>
    {result && !result.online && <p className="notice" role="status">
      {result.catalog ? '正在显示上次验签成功的目录。' : '暂时无法读取官方目录。'}
      {result.error && (' ' + result.error)}
    </p>}
    {result?.catalog && <p className="gallery-revision muted">目录修订号 {result.catalog.revision}</p>}
    {result?.catalog?.widgets.length === 0 && <p className="empty">暂无已收录的组件</p>}
    <WidgetBrowser items={entries}>
      {entry => {
        const local = packages.find(item => item.id === entry.id);
        const count = instances.filter(item => item.packageId === entry.id).length;
        const published = publishedReleases(entry);
        const selected = compatibleRelease(published);
        const minimum = published.reduce<string | undefined>((lowest, item) =>
          !lowest || compareVersion(item.minHostVersion, lowest) < 0 ? item.minHostVersion : lowest, undefined);
        const comparison = local && selected ? compareVersion(local.version, selected.version) : -1;
        const same = comparison === 0 && local?.digest === selected?.sha256;
        const conflict = comparison === 0 && !same;
        const online = result?.online && !loading;
        return <article className="package gallery-package" key={entry.id}>
          <span className="badge">{entry.category}</span><h2>{entry.name}</h2>
          <p>{entry.summary}</p>
          {selected && <small>v{selected.version} · SoulDesk {selected.minHostVersion} 或以上</small>}
          {selected && <small>权限 · {permissionText(selected.capabilities)}</small>}
          {!selected && <small>{minimum ? '需要 SoulDesk ' + minimum + ' 或以上' : '暂无可安装版本'}</small>}
          {local && <small>本地 v{local.version} · {count} 个实例</small>}
          {comparison > 0 && <small>本地版本较新</small>}
          {conflict && <small className="error">同版本的本地内容与官方发布包冲突</small>}
          <div className="gallery-actions actions">
            {selected && comparison < 0 && <button className="primary" disabled={!online || unavailable}
              onClick={() => setConfirmation({ entry, release: selected, local })}>{local ? '更新' : '安装'}</button>}
            {same && <button className="primary" disabled={unavailable || !canAdd} onClick={() => onAdd(entry.id)}>添加到面板</button>}
            {local && (count ? <button onClick={() => onManage(entry.id)}>管理实例</button>
              : <button disabled={unavailable} onClick={() => onUninstall(entry.id)}>卸载</button>)}
          </div>
        </article>;
      }}
    </WidgetBrowser>
    {confirmation && <ConfirmRelease target={confirmation} onClose={() => setConfirmation(undefined)} onInstalled={onRefresh} />}
  </>;
}
