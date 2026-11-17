import { computed, ref } from 'vue';
import dayjs from 'dayjs';
import { useWeatherContext } from '../context';
import { WeatherUtils } from '../utils/WeatherUtils';
import { temperature, forecastDayText as labelForecast } from '../weather-model.js';
export function useQWeatherApi(_option?: unknown) {
  const { model, settings, clock } = useWeatherContext();
  const now = ref(dayjs());
  const weatherData = computed(() => {
    const value = model.state.now;
    return value ? { ...value, windScale:value.windScale ?? '—', windDir:value.windDir ?? '—', windSpeed:value.windSpeed ?? '—', humidity:value.humidity ?? '—', precip:value.precip ?? '—', pressure:value.pressure ?? '—' } : undefined;
  });
  return {
    now, weatherData,
    errorMsg: computed(() => Object.values(model.state.errors).join('；')),
    backgroundClass: computed(() => WeatherUtils.getBackgroundClass(Number(weatherData.value?.icon || '100'))),
    selectLocation: computed(() => ({ id:settings.value.cityId, name:settings.value.cityName })),
    unitText: computed(() => settings.value.unit === 'F' ? '°F' : '°C'),
    temperature: computed(() => temperature(weatherData.value?.temp, settings.value.unit)),
    dailyIndex: computed(() => model.state.indices[Math.floor(Math.random()*model.state.indices.length)]),
    forecastDayText: (date:string) => labelForecast(date,settings.value.timezone,clock.value),
    weather3dResponse: computed(() => model.state.daily ? { daily:model.state.daily.map((day: any) => ({ ...day, tempMin:temperature(day.tempMin, settings.value.unit) ?? '—', tempMax:temperature(day.tempMax, settings.value.unit) ?? '—' })) } : undefined),
  };
}
