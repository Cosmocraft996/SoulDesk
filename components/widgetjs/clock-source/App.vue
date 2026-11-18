<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { ComponentFrame, settings, status } from './souldesk';
import Clock from './widgets/clock/ClockWidgetView.vue';
import Flip from './widgets/flip-clock/FlipClockWidgetView.vue';
import Glitch from './widgets/glitch/GlitchWidgetView.vue';
import Mickey from './widgets/mickey-clock/MickeyClockWidgetView.vue';
import TextClock from './widgets/text-clock/TextClockWidgetView.vue';
import timezoneData from './i18n/timezone.json';
const props = defineProps<{ host: any; variant: string }>();
const views: Record<string, any> = { clock: Clock, 'flip-clock': Flip, 'glitch-clock': Glitch, 'mickey-clock': Mickey, 'text-clock': TextClock };
const view = computed(() => views[props.variant]);
const dialog = ref<HTMLDialogElement>();
const draft = ref<Record<string, any>>({});
const saving = ref(false), saveError = ref('');
const timezones = timezoneData.flatMap(region => region.timezones).map(item => ({ value: item.value, label: item.label.zh }));
function openSettings() {
  draft.value = { ...settings };
  saveError.value = '';
  dialog.value?.showModal();
}
async function save(event: Event) {
  event.preventDefault();
  saving.value = true; saveError.value = '';
  try {
    const next = { ...draft.value, radius: Number(draft.value.radius), fontSize: Number(draft.value.fontSize) };
    if (next.timezone) new Intl.DateTimeFormat('zh-CN', { timeZone: next.timezone }).format();
    const result = await props.host.request('settings.update', next);
    Object.assign(settings, result); dialog.value?.close();
  } catch (error: any) { saveError.value = '保存失败：' + error.message; }
  finally { saving.value = false; }
}
const theme = computed(() => ({
  '--widget-background-color': settings.background,
  '--widget-color': settings.foreground,
  '--widget-border-radius': settings.radius + 'px', '--widget-font-size': settings.fontSize + 'px',
  '--widget-font-family': settings.font || 'FiraCode',
}));
onMounted(async () => {
  await nextTick(); await props.host.ready();
  if (props.host.preview || new URLSearchParams(location.search).get('verify') !== 'widgetjs') return;
  const old = await props.host.request('storage.get');
  const previous = old.verification;
  let invalidSaveRejected = previous?.invalidSaveRejected === true;
  if (!previous?.uiVerified) {
    openSettings(); await nextTick();
    const before = JSON.stringify(await props.host.request('settings.get'));
    draft.value.foreground = 'invalid-color';
    await save(new Event('submit'));
    invalidSaveRejected = JSON.stringify(await props.host.request('settings.get')) === before && saveError.value.startsWith('保存失败');
    if (!invalidSaveRejected) throw new Error('时钟无效配置未被拒绝');
    draft.value.foreground = settings.foreground;
    draft.value.background = '#' + props.host.instanceId.replaceAll('-', '').slice(0, 6);
    draft.value.radius = 12;
    await nextTick();
    const form = document.getElementById('clock-form') as HTMLFormElement;
    form.requestSubmit();
    for (let n = 0; n < 100 && saving.value; n++) await new Promise(resolve => setTimeout(resolve, 50));
    if (saving.value || dialog.value?.open) throw new Error('时钟表单保存未完成：' + saveError.value);
  }
  await nextTick();
  // 等待真实表盘图像完成加载，不能用Vue已挂载代替资源验收。
  const images = [...document.querySelectorAll<HTMLImageElement>('#clock-root img')];
  await Promise.all(images.map(image => image.complete ? Promise.resolve() : new Promise<void>(resolve => { image.addEventListener('load', () => resolve(), { once: true }); image.addEventListener('error', () => resolve(), { once: true }); setTimeout(resolve, 3000); })));
  const saved = await props.host.request('settings.get');
  await props.host.request('storage.set', { ...old, verification: {
    uiVerified: true, invalidSaveRejected, runtimeLoads: (previous?.runtimeLoads ?? 0) + 1,
    stylesLoaded: getComputedStyle(document.getElementById('clock-root')!).borderRadius === '12px' && getComputedStyle(document.getElementById('clock-settings')!).position === 'absolute',
    imagesLoaded: images.every(image => image.complete && image.naturalWidth > 0),
    configurationSaved: saved.background === '#' + props.host.instanceId.replaceAll('-', '').slice(0, 6),
    savedLabel: saved.background, renderedText: document.querySelector('.component-frame')?.textContent?.slice(0, 300),
    origin: location.origin,
  } });
});
</script>
<template>
  <div id="clock-root" :style="theme">
    <component :is="view" v-if="view" />
    <button id="clock-settings" type="button" aria-label="设置时钟" @click="openSettings">⚙</button>
    <output id="clock-status" :class="{ error: status.error }" role="status">{{ status.message }}</output>
    <dialog ref="dialog" id="clock-dialog"><form id="clock-form" @submit="save"><h2>时钟设置</h2>
      <label v-if="'timezone' in draft">时区<select v-model="draft.timezone"><option v-for="item in timezones" :key="item.value" :value="item.value">{{ item.label }}</option></select></label>
      <label v-if="'showTimezone' in draft">显示时区<input v-model="draft.showTimezone" type="checkbox"></label>
      <label v-if="'font' in draft">字体<input v-model="draft.font" maxlength="80"></label>
      <label>背景<input v-model="draft.background" type="color"></label><label>文字<input v-model="draft.foreground" type="color"></label>
      <label>字号<input v-model="draft.fontSize" type="number" min="12" max="40" required></label>
      <label>圆角<input v-model="draft.radius" type="number" min="0" max="40" required></label>
      <p role="status">{{ saveError }}</p><footer><button type="button" @click="dialog?.close()">取消</button><button :disabled="saving" type="submit">{{ saving ? '保存中' : '保存' }}</button></footer>
    </form></dialog>
  </div>
</template>
