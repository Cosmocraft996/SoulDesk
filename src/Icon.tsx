import type { SVGProps } from 'react';

// 自绘线条图标，不引入组件库；可操作按钮在使用处提供中文名称。
const paths = {
  plus: 'M12 5v14M5 12h14',
  settings: 'M9 3h6l1 4 4 2v6l-4 2-1 4H9l-1-4-4-2V9l4-2 1-4ZM9 12a3 3 0 1 0 6 0 3 3 0 0 0-6 0',
  close: 'm6 6 12 12M18 6 6 18',
  collapse: 'm9 5 7 7-7 7',
  grip: 'M8 5h.01M16 5h.01M8 12h.01M16 12h.01M8 19h.01M16 19h.01',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  resize: 'm9 19 10-10M15 19l4-4',
  desktop: 'M3 4h18v13H3V4ZM8 21h8M12 17v4',
  panel: 'M3 4h18v16H3V4ZM15 4v16',
  pin: 'm8 3 8 0-1 6 4 4H5l4-4-1-6ZM12 13v8',
  hide: 'm3 3 18 18M10 10a3 3 0 0 0 4 4M4 8l-2 4s4 6 10 6l4-1M9 6l3-1c6 0 10 7 10 7l-2 3',
  detail: 'M4 3h16v18H4V3ZM8 8h8M8 12h8M8 16h5',
  reload: 'M20 7V3l-4 4M20 7a8 8 0 1 0 0 10M20 7h-5',
  download: 'M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5',
} as const;
export type IconName = keyof typeof paths;
export function Icon({ name, ...props }: SVGProps<SVGSVGElement> & { name: IconName }) {
  return <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={name === 'grip' || name === 'more' ? 3 : 1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}><path d={paths[name]} /></svg>;
}
