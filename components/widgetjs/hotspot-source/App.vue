<script setup lang="ts">
import { computed, ref, onMounted, onBeforeUnmount } from 'vue';
import { verifyHotspot } from './verify.js';
import { fallbackRemoteImage } from './images.js';
import Bangumi from './widgets/bangumi/BangumiWidgetView.vue';
import Bilibili from './widgets/bilibili/BilibiliWidgetView.vue';
import Rcmd from './widgets/bilibili-rcmd/BilibiliRcmdWidgetView.vue';
import Cls from './widgets/cls-telegraph/ClsTelegraphWidgetView.vue';
import Douyin from './widgets/douyin/DouyinWidgetView.vue';
import Toutiao from './widgets/tou-tiao/TouTiaoWidgetView.vue';
import Weibo from './widgets/weibo/WeiBoWidgetView.vue';
import Zhihu from './widgets/zhihu/ZhihuWidgetView.vue';
import { contentStatus, settings, imageStatus } from './native';
const props = defineProps<{ host: any }>();
const views: Record<string, any> = { bangumi: Bangumi, bilibili: Bilibili, 'bilibili-rcmd': Rcmd, 'cls-telegraph': Cls, douyin: Douyin, 'tou-tiao': Toutiao, weibo: Weibo, zhihu: Zhihu };
const view = views[document.body.dataset.variant || ''];
if (!view) throw new Error('未知热点组件');
const dialog = ref<HTMLDialogElement>(), draft = ref<Record<string, any>>({}), saving = ref(false), error = ref('');
let alive = true;
onMounted(() => void verifyHotspot(props.host, () => ({ alive, saving: saving.value })));
onBeforeUnmount(() => { alive = false; });
const theme = computed(() => ({ '--widget-background-color': settings.background, '--widget-color': settings.foreground, borderRadius: settings.radius + 'px', background: settings.background, color: settings.foreground }));
function edit() { draft.value = { ...settings }; error.value = ''; dialog.value?.showModal(); }
async function save(event: Event) {
  event.preventDefault(); if (saving.value) return;
  saving.value = true;
  try {
    const value = draft.value;
    if (!/^#[0-9a-f]{6}$/i.test(value.background) || !/^#[0-9a-f]{6}$/i.test(value.foreground)) throw new Error('颜色格式无效');
    Object.assign(settings, await props.host.request('settings.update', { background: value.background, foreground: value.foreground, radius: Number(value.radius) }));
    dialog.value?.close();
  } catch (e: any) { error.value = '保存失败：' + e.message; }
  finally { saving.value = false; }
}
</script>
<template>
  <main id="hotspot-root" :style="theme" @error.capture="fallbackRemoteImage($event, imageStatus)"><div class="content-view"><component :is="view" /></div><button class="settings-button" type="button" aria-label="设置热点组件" @click="edit">⚙</button>
    <footer class="content-status" :class="{ error: contentStatus.error }" role="status">{{ host.preview ? '预览 · 不读取网络' : contentStatus.error ? '获取失败：' + contentStatus.error + (contentStatus.updatedAt ? ' · 当前为上次成功数据' : '') : contentStatus.loading ? '正在更新…' : contentStatus.updatedAt ? '更新于 ' + new Date(contentStatus.updatedAt).toLocaleTimeString() : '等待公开数据' }}<span v-if="imageStatus.failed"> · {{ imageStatus.failed }}张图片未加载</span></footer>
    <dialog ref="dialog"><form @submit="save"><h2>组件设置</h2><label>背景颜色<input v-model="draft.background" type="color"></label><label>文字颜色<input v-model="draft.foreground" type="color"></label><label>圆角<input v-model="draft.radius" type="number" min="0" max="40" required></label><output role="status">{{ error }}</output><div class="actions"><button type="button" @click="dialog?.close()">取消</button><button type="submit" :disabled="saving">{{ saving ? '保存中' : '保存' }}</button></div></form></dialog>
  </main>
</template>
