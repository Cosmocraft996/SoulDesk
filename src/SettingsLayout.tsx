import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import {
  ArrowRight, Bell, Database, Info, Keyboard, LayoutGrid, Palette, PanelRight,
  Search, SlidersHorizontal, X, type LucideIcon,
} from 'lucide-react';
import logo from '../src-tauri/icons/icon.svg';
import lightLogo from '../src-tauri/icons/icon-light.svg';

const settings = {
  general: { label: '常规', icon: SlidersHorizontal, description: '管理 SoulDesk 的启动方式和桌面组件的显示状态。' },
  appearance: { label: '外观', icon: Palette, description: '为宿主界面选择主题，调整组件透明区域的衬底。' },
  desktop: { label: '桌面与侧栏', icon: PanelRight, description: '调整侧栏的展开方式、挂件样式和桌面格子。' },
  shortcuts: { label: '快捷键', icon: Keyboard, description: '为侧栏和灵动通知设置全局组合键。' },
  notifications: { label: '通知与声音', icon: Bell, description: '管理通知渠道、灵动通知、行情铃声和历史记录。' },
  widgets: { label: '组件管理', icon: LayoutGrid, description: '管理桌面和侧栏中的组件，安装或导入新组件。' },
  data: { label: '网络与数据', icon: Database, description: '管理公开行情连接，查看运行状态并备份本地数据。' },
  about: { label: '关于与更新', icon: Info, description: '查看版本、更新状态和开源许可。' },
} satisfies Record<string, { label: string; icon: LucideIcon; description: string }>;
type Page = keyof typeof settings;
const widgetTabs = { instances: '已添加', library: '组件库', gallery: '官方广场' } as const;
type WidgetTab = keyof typeof widgetTabs;
export type SettingsTab = Exclude<Page, 'widgets'> | WidgetTab;

export const isWidgetTab = (tab: SettingsTab): tab is WidgetTab => Object.hasOwn(widgetTabs, tab);
export function settingsTab(value: string | null): SettingsTab {
  return value && value !== 'widgets' && (Object.hasOwn(settings, value) || Object.hasOwn(widgetTabs, value))
    ? value as SettingsTab : 'general';
}
const navigation: { label: string; pages: Page[] }[] = [
  { label: '偏好设置', pages: ['general', 'appearance', 'desktop', 'shortcuts', 'notifications'] },
  { label: '管理', pages: ['widgets', 'data', 'about'] },
];
const pageTab = (page: Page): SettingsTab => page === 'widgets' ? 'instances' : page;
const pageOf = (tab: SettingsTab): Page => isWidgetTab(tab) ? 'widgets' : tab;
type Destination = { tab: SettingsTab; label: string; id?: string; terms?: string };
const destinations: Destination[] = [
  ...navigation.flatMap(group => group.pages.map(page => ({ tab: pageTab(page), label: settings[page].label }))),
  { tab: 'general', label: '开机启动', id: 'autostart', terms: '自启 自动启动 登录' },
  { tab: 'general', label: '静默启动', id: 'silent-startup', terms: '后台' },
  { tab: 'general', label: '组件显示', id: 'widget-visibility', terms: '隐藏全部 恢复显示' },
  { tab: 'general', label: '打开侧栏', id: 'side-panel-toggle', terms: '组件面板 开关 收起' },
  { tab: 'appearance', label: '应用主题', id: 'app-theme', terms: '浮格蓝 深色 浅色 墨蓝 松烟 燕麦 雾紫 配色' },
  { tab: 'appearance', label: '宿主衬底', id: 'widget-backdrop', terms: '组件背景' },
  { tab: 'desktop', label: '侧栏偏好', id: 'panel-preferences', terms: '面板 宽度 遮罩 透明度 滑入 滑出 挂件 双击' },
  { tab: 'desktop', label: '桌面格子', id: 'desktop-grid', terms: '整理 文件 Windows 桌面图标 恢复' },
  { tab: 'desktop', label: '找回屏幕外组件', id: 'repair-windows', terms: '显示器 多屏' },
  { tab: 'shortcuts', label: '侧栏快捷键', id: 'panel-shortcuts', terms: 'Shift+Space 面板 键位 冲突' },
  { tab: 'shortcuts', label: '灵动通知快捷键', id: 'notification-shortcuts', terms: 'Alt+H Alt+M 显隐 静音 键位' },
  { tab: 'notifications', label: '通知渠道', id: 'notification-channels', terms: 'Windows 系统通知 测试' },
  { tab: 'notifications', label: '灵动通知', id: 'notification-island', terms: '透明度 音量 显隐 静音 条数 模拟来电 测试' },
  { tab: 'notifications', label: '行情铃声', id: 'notification-sounds', terms: '上涨 下跌 试听 声音' },
  { tab: 'notifications', label: '灵动通知历史', id: 'notification-history', terms: '清空 保留天数 排队' },
  { tab: 'instances', label: '已添加的组件', terms: '实例 显示 隐藏 桌面 侧栏' },
  { tab: 'library', label: '组件库', terms: '安装 导入 ZIP 本地 组件包' },
  { tab: 'gallery', label: '官方广场', terms: '安装 下载 目录' },
  { tab: 'data', label: '公开行情网络', id: 'market-network', terms: '代理 系统 直连 HTTP SOCKS5 重连 监控缺口' },
  { tab: 'data', label: '本地数据备份', id: 'backup', terms: '数据库' },
  { tab: 'data', label: '本地数据与诊断', id: 'local-data', terms: '目录 路径 窗口 会话 测试声音' },
  { tab: 'about', label: '应用更新', id: 'app-updates', terms: '检查 自动更新 升级 版本' },
  { tab: 'about', label: '开源许可', id: 'licenses', terms: 'Lucide 图标' },
];

export function SettingsLayout({ tab, onNavigate, version, connected, notice, children }: {
  tab: SettingsTab; onNavigate: (tab: SettingsTab) => void; version: string; connected: boolean;
  notice?: ReactNode; children: ReactNode;
}) {
  const page = pageOf(tab);
  const view = settings[page];
  const scroller = useRef<HTMLElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  const [jump, setJump] = useState<{ tab: SettingsTab; id?: string }>();
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  const results = (words.length ? destinations.filter(item => {
    const text = `${item.label} ${item.terms ?? ''} ${settings[pageOf(item.tab)].label}`.toLowerCase();
    return words.every(word => text.includes(word));
  }) : destinations.slice(0, 8)).slice(0, 18);

  const navigate = (tab: SettingsTab, id?: string) => {
    onNavigate(tab);
    setJump({ tab, id });
  };
  useLayoutEffect(() => { scroller.current?.scrollTo?.({ top: 0 }); }, [tab]);
  useLayoutEffect(() => {
    if (jump?.tab !== tab) return;
    const target = jump.id ? document.getElementById(jump.id) : null;
    const heading = target ?? document.getElementById('settings-page-title');
    heading?.focus({ preventScroll: true });
    heading?.scrollIntoView?.({ block: target ? 'center' : 'start' });
    setJump(undefined);
  }, [jump, tab]);

  const openSearch = () => {
    if (!dialog.current || dialog.current.open || document.querySelector('dialog[open]')) return;
    setQuery('');
    dialog.current.showModal();
    search.current?.focus();
  };
  useEffect(() => {
    const openWithShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || !event.ctrlKey || event.altKey || event.shiftKey
        || event.key.toLowerCase() !== 'k' || document.querySelector('dialog[open]')) return;
      event.preventDefault();
      openSearch();
    };
    window.addEventListener('keydown', openWithShortcut);
    return () => window.removeEventListener('keydown', openWithShortcut);
  }, []);
  const choose = (result: Destination) => {
    dialog.current?.close();
    navigate(result.tab, result.id);
  };

  return <>
    <header className="settings-header">
      <div className="settings-brand">
        <img className="settings-brand-default" src={logo} alt="" />
        <img className="settings-brand-light" src={lightLogo} alt="" />
        <strong>SoulDesk</strong>
      </div>
      <button className="settings-search-trigger" type="button" aria-haspopup="dialog" aria-keyshortcuts="Control+K" onClick={openSearch}>
        <Search aria-hidden /><span>搜索设置</span><kbd>Ctrl + K</kbd>
      </button>
    </header>
    <aside className="settings-sidebar">
      <nav aria-label="设置分类">{navigation.map(group => <div className="settings-nav-group" key={group.label}>
        <p>{group.label}</p>
        {group.pages.map(id => {
          const { label, icon: Glyph } = settings[id];
          return <button key={id} type="button" aria-current={page === id ? 'page' : undefined} onClick={() => navigate(pageTab(id))}>
            <Glyph aria-hidden /><span>{label}</span>
          </button>;
        })}
      </div>)}</nav>
      <div className="settings-local"><i className={connected ? 'dot' : 'dot offline'} aria-hidden />{connected ? '本地运行' : '等待桌面宿主'}<small>{version}</small></div>
    </aside>
    <section className="management-content" aria-labelledby="settings-page-title" ref={scroller}>
      <div className="content">
        <header className="settings-page-heading"><h1 id="settings-page-title" tabIndex={-1}>{view.label}</h1><p>{view.description}</p></header>
        {notice}
        {isWidgetTab(tab) && <nav className="settings-widget-tabs" aria-label="组件管理分类">
          {Object.entries(widgetTabs).map(([id, label]) => <button key={id} type="button" aria-current={tab === id ? 'true' : undefined}
            onClick={() => onNavigate(id as WidgetTab)}>{label}</button>)}
        </nav>}
        {children}
      </div>
    </section>
    <dialog className="settings-search" ref={dialog} aria-label="搜索设置" onClick={event => { if (event.target === dialog.current) dialog.current?.close(); }}
      onKeyDown={event => {
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
        const buttons = Array.from(dialog.current?.querySelectorAll<HTMLButtonElement>('.settings-result') ?? []);
        if (!buttons.length) return;
        event.preventDefault();
        const current = buttons.findIndex(button => button === document.activeElement);
        const step = event.key === 'ArrowDown' ? 1 : -1;
        buttons[current < 0 ? (step > 0 ? 0 : buttons.length - 1) : (current + step + buttons.length) % buttons.length]?.focus();
      }}>
      <form className="settings-search-field" onSubmit={event => { event.preventDefault(); if (results[0]) choose(results[0]); }}>
        <Search aria-hidden /><input ref={search} type="search" value={query} onChange={event => setQuery(event.target.value)}
          aria-label="搜索设置项" placeholder="搜索主题、侧栏或代理…" />
        <button type="button" aria-label="关闭搜索" onClick={() => dialog.current?.close()}><X aria-hidden /></button>
      </form>
      <div className="settings-results">{results.length ? results.map(item => {
        const group = settings[pageOf(item.tab)];
        const Glyph = group.icon;
        return <button className="settings-result" key={item.label} type="button" onClick={() => choose(item)}>
          <Glyph aria-hidden /><span>{item.label}</span>{item.label !== group.label && <small>{group.label}</small>}
        </button>;
      }) : <p role="status">未找到相关设置。</p>}</div>
    </dialog>
  </>;
}

export function SettingsGroup({ id, title, children }: { id?: string; title: string; children: ReactNode }) {
  return <section className="settings-group" id={id} tabIndex={id ? -1 : undefined}>
    <h2>{title}</h2><div className="settings-rows">{children}</div>
  </section>;
}

export function SettingsRow({ id, icon: Glyph, title, description, children }: {
  id?: string; icon: LucideIcon; title: string; description?: string; children: ReactNode;
}) {
  return <div className="settings-item" id={id} tabIndex={id ? -1 : undefined}>
    <span className="settings-item-icon"><Glyph aria-hidden /></span>
    <div><strong>{title}</strong>{description && <p>{description}</p>}</div>
    <div className="settings-item-actions">{children}</div>
  </div>;
}

export function SettingsSwitch({ label, checked, disabled, onChange }: {
  label: string; checked: boolean; disabled?: boolean; onChange: (checked: boolean) => void;
}) {
  return <label className="settings-switch">
    <input type="checkbox" role="switch" aria-label={label} checked={checked} disabled={disabled} onChange={event => onChange(event.target.checked)} />
    <span aria-hidden="true" />
  </label>;
}

export function DesktopSettingsLink({ onClick }: { onClick: () => void }) {
  return <section className="settings-desktop-link">
    <div><h2>桌面与侧栏</h2><p>管理桌面格子、侧栏和桌面挂件。</p>
      <button className="primary" type="button" onClick={onClick}>打开设置<ArrowRight aria-hidden /></button>
    </div>
    <div className="settings-desktop-art" aria-hidden="true">
      <div><b>09:41</b><span>星期三</span></div>
      <div><i /><i /><i /><i /><i /><i /></div>
      <div><i /><i /><i /></div>
    </div>
  </section>;
}
