import { inject, type InjectionKey } from 'vue';
export const weatherContext: InjectionKey<any> = Symbol('SoulDesk天气实例');
export function useWeatherContext() {
  const value = inject(weatherContext);
  if (!value) throw Error('天气实例上下文缺失');
  return value;
}
export function useWidget() { return useWeatherContext(); }
