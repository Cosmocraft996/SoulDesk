import { computed } from 'vue';
import dayjs from 'dayjs';
import utc from 'dayjs/plugin/utc';
import timezonePlugin from 'dayjs/plugin/timezone';
import { useInstanceSetting } from '../souldesk';
import timezones from '../i18n/timezone.json';
dayjs.extend(utc);
dayjs.extend(timezonePlugin);
export function useComponentTimeZone() {
  const timezone = useInstanceSetting('timezone', 'Asia/Shanghai');
  const timezoneLabel = computed(() => timezones.flatMap(region => region.timezones).find(item => item.value === timezone.value)?.label.zh ?? timezone.value);
  return { timezone, timezoneLabel, getNow: () => dayjs().tz(timezone.value) };
}
