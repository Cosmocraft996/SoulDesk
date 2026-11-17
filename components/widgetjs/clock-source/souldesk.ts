import { computed, defineComponent, h, reactive } from 'vue';
export const settings = reactive<Record<string, any>>({});
export const status = reactive({ message: '', error: false });
export function useInstanceSetting<T>(key: string, fallback: T) {
  return computed<T>({ get: () => settings[key] ?? fallback, set: value => { settings[key] = value; } });
}
export const ComponentFrame = defineComponent({
  name: 'ComponentFrame',
  setup(_, { slots }) { return () => h('section', { class: 'component-frame' }, slots.default?.()); },
});
export function useComponent() { return { settings }; }
export function useComponentTheme() { return { settings }; }
let messageTimer: ReturnType<typeof setTimeout>;
export function showMessage(message: string) {
  clearTimeout(messageTimer); status.message = message; status.error = false;
  messageTimer = setTimeout(() => { status.message = ''; }, 4000);
}
