import { useRef, useState, type ReactNode } from 'react';
import { Icon } from './Icon';
import bundledCategories from './widget-categories.json';

const other = '其他';
const categoryOrder = Object.keys(bundledCategories);
// 随宿主提供已收录组件的分类，兼容未联网的旧安装和旧目录。
const bundledById = new Map(Object.entries(bundledCategories).flatMap(
  ([category, ids]) => ids.map(id => [id, category] as const),
));

export function widgetCategory(id: string, category?: string) {
  const label = category?.trim();
  return label && label !== '桌面组件' ? label : bundledById.get(id) ?? other;
}

interface BrowseItem {
  id: string;
  name: string;
  category: string;
  summary?: string;
}

export function WidgetBrowser<T extends BrowseItem>({ items, children }: {
  items: T[];
  children: (item: T) => ReactNode;
}) {
  const [category, setCategory] = useState('');
  const [query, setQuery] = useState('');
  const search = useRef<HTMLInputElement>(null);
  const available = new Set(items.map(item => item.category));
  const categories = [
    ...categoryOrder.filter(label => available.delete(label)),
    ...[...available].filter(label => label !== other).sort((a, b) => a.localeCompare(b, 'zh-CN')),
    ...(available.has(other) ? [other] : []),
  ];
  const selected = categories.includes(category) ? category : '';
  const keyword = query.trim().toLowerCase();
  const visible = items.filter(item =>
    (!selected || item.category === selected)
    && (!keyword || [item.name, item.summary, item.category, item.id]
      .some(value => value?.toLowerCase().includes(keyword))),
  );

  if (!items.length) return null;
  return <>
    <div className="widget-filters">
      <label className="widget-search">
        <Icon name="search" />
        <input ref={search} type="search" aria-label="搜索组件" placeholder="搜索组件" value={query}
          onChange={event => setQuery(event.target.value)} />
      </label>
      <div className="widget-categories" role="group" aria-label="组件分类">
        <button type="button" aria-pressed={!selected} onClick={() => setCategory('')}>全部</button>
        {categories.map(label => <button key={label} type="button" aria-pressed={label === selected}
          onClick={() => setCategory(label)}>{label}</button>)}
      </div>
    </div>
    <p className="widget-result-count muted" role="status">{visible.length} 个组件</p>
    {visible.length ? <div className="library-grid">{visible.map(children)}</div>
      : <div className="empty widget-empty">
        <p>没有符合条件的组件</p>
        <button onClick={() => { setQuery(''); setCategory(''); search.current?.focus(); }}>清除筛选</button>
      </div>}
  </>;
}
