import { defineComponent, h, reactive } from 'vue';
import { useWindowSize } from '@vueuse/core';
export let host: any;
export const contentStatus = reactive({ loading: 0, error: '', updatedAt: 0 });
// 仅显式隔离验收写入网络结果摘要，不保存热点正文。
export const hotspotEvidence = reactive<Record<string, any>>({});
export const imageStatus = reactive({ failed: 0 });
export const settings = reactive<Record<string, any>>({});
export function setHost(value: any) { host = value; }
export function contentFailure(error: any) { contentStatus.error = String(error?.message || error); }
export async function openExternal(value: string, _options?: unknown) {
  if (host.preview) return;
  try { await host.request('links.open', { url: value.startsWith('//') ? 'https:' + value : value }); }
  catch (error) { contentFailure(error); }
}
export function secureImage(value: string) {
  if (!value) return '';
  if (value.startsWith('data:image/')) return value;
  if (value.startsWith('//')) return 'https:' + value;
  if (value.startsWith('http://')) return 'https://' + value.slice(7);
  return value.startsWith('https://') ? value : '';
}
export function useComponent() { return { settings }; }
export function useComponentSize() { return useWindowSize(); }
export function currentViewport() {
  const { width, height } = useWindowSize();
  return { get widthPx() { return width.value; }, get heightPx() { return height.value; } };
}
export const ComponentFrame = defineComponent({
  name: 'ComponentFrame',
  setup(_, { slots }) { return () => h('section', { class: 'component-frame' }, slots.default?.()); },
});
export const ScrollArea = defineComponent({
  name: 'ScrollArea', props: ['height', 'wrapStyle'],
  setup(props, { slots }) { return () => h('div', { class: 'scroll-area', style: [props.wrapStyle, { height: typeof props.height === 'number' ? Math.max(0, props.height) + 'px' : props.height, overflowY: 'auto', overflowX: 'hidden' }] }, slots.default?.()); },
});
