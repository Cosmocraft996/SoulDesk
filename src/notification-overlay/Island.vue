<!-- 由固定原模板派生，来源与散列见 SOURCE_FILES.json。 -->
<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import MessageNotification from './parts/Message.vue';
import ReminderNotification from './parts/Reminder.vue';
import AdvanceCountdownNotification from './parts/Advance.vue';
import CountingNotification from './parts/Counting.vue';
import PhoneCallNotification from './parts/Phone.vue';
import Icon from './Icon.vue';

const props = defineProps({ host: Object, notification: Object, height: Number, sampledAt: Number, audible: Boolean, audioMuted: Boolean, audioVolume: { type: Number, default: 1 }, audioError: String });
const error = ref('');
const deletingRule = ref(false);
const elapsed = ref(0);
const startAt = props.sampledAt;
const now = computed(() => startAt + elapsed.value);
const kind = computed(() => props.notification.kind);
const market = computed(() => props.notification.marketAlert);
const replayTime = computed(() => market.value?.replayed ? new Date(market.value.triggeredAt) : null);
const replayLabel = computed(() => replayTime.value?.toLocaleString('zh-CN', {
  hour12: false, fractionalSecondDigits: 3, second: '2-digit', minute: '2-digit',
  hour: '2-digit', year: 'numeric', month: '2-digit', day: '2-digit',
}));
let timer, drag;

async function request(method, args = {}) {
  if (method !== 'hover') error.value = '';
  try {
    return await props.host.request(`island.${method}`, { ...args, id: props.notification.id });
  } catch (failure) {
    if (method !== 'hover') error.value = failure?.message ?? String(failure);
  }
}

async function deleteRule() {
  if (deletingRule.value) return;
  deletingRule.value = true;
  await request('market-delete-rule');
  deletingRule.value = false;
}

function startDrag(event) {
  if (event.button !== 0 || !event.isPrimary) return;
  drag = request('drag');
}

async function openNotice(event) {
  const attempt = drag;
  drag = undefined;
  if (event.detail !== 0 && attempt && await attempt !== false) return;
  if (market.value) await request('action', { action: 'open-source' });
  else await request('history');
}

onMounted(() => {
  if (!['advance-countdown', 'countdown'].includes(kind.value)) return;
  const start = performance.now();
  timer = setInterval(() => { elapsed.value = performance.now() - start; }, kind.value === 'countdown' ? 10 : 1000);
});
onBeforeUnmount(() => clearInterval(timer));
</script>

<template>
  <article class="island" :class="kind" :style="{ height: `${height}px` }"
    @mouseenter="request('hover', { entered: true })" @mouseleave="request('hover', { entered: false })">
    <button class="history-hit" type="button" :aria-label="market ? '查看 K 线' : '查看通知历史'"
      :title="market ? `${notification.title} · 警报价 ${market.threshold} · ${notification.message}${replayLabel ? ` · 重启补报 · 触发于 ${replayLabel}` : ''}` : undefined"
      @pointerdown="startDrag" @click="openNotice" />
    <div v-if="market" class="market-card">
      <div class="market-instrument">
        <strong>{{ market.key.symbol.replace(/USDT$/, '/USDT') }}</strong>
        <small>{{ market.demo ? '测试 · ' : '' }}{{ market.key.market === 'spot' ? '现货' : '永续' }} · {{ market.key.priceKind === 'mark' ? '标记价' : '成交价' }}</small>
      </div>
      <div class="market-result">
        <strong class="market-move" :class="market.direction">{{ notification.message }}</strong>
        <small v-if="replayTime">补报 {{ replayTime.toLocaleTimeString('zh-CN', { hour12: false }) }}</small>
      </div>
    </div>
    <CountingNotification v-else-if="kind === 'countdown'" v-bind="notification" :now="now" />
    <AdvanceCountdownNotification v-else-if="kind === 'advance-countdown'" v-bind="notification" :now="now" @action="request('action', { action: $event })" />
    <ReminderNotification v-else-if="kind === 'reminder'" :notification="notification" @action="request('action', { action: $event })" />
    <PhoneCallNotification v-else-if="kind === 'call'" :host="host" :notification="notification" :audible="audible" :audio-muted="audioMuted" :audio-volume="audioVolume" @action="request('action', { action: $event })" @error="error = $event" />
    <MessageNotification v-else :type="kind" :message="notification.message" :title="notification.title" :source="notification.sourceName" />
    <button v-if="market" class="island-close island-delete" type="button"
      :aria-label="market.demo ? '模拟删除警报规则' : '删除此价格警报规则'"
      :title="market.demo ? '模拟删除警报规则' : '删除此价格警报规则'"
      :disabled="deletingRule" @click="deleteRule">
      <Icon type="trash" />
    </button>
    <button v-if="market" class="island-close island-mute" type="button"
      aria-label="静音这条行情通知" :aria-pressed="notification.mute"
      :title="notification.mute ? '恢复这条通知的声音' : '静音这条通知'"
      @click="request('market-mute', { muted: !notification.mute })">
      <Icon :type="notification.mute ? 'volume-off' : 'volume'" />
    </button>
    <button v-if="kind !== 'reminder' && kind !== 'call'" class="island-close" type="button" aria-label="关闭通知" @click="request('action', { action: 'dismiss' })">×</button>
    <span v-if="error || audioError" class="display-error" role="alert">{{ error || audioError }}</span>
  </article>
</template>

<style scoped>
.island {
  position: relative;
  display: flex;
  flex: none;
  width: min(326px, 100%);
  margin-inline: auto;
  overflow: hidden;
  border-radius: 42px;
  background: #0a0a0a;
  color: white;
  user-select: none;
  -webkit-font-smoothing: antialiased;
}
.island:is(.info, .success, .error, .warning, .countdown, .market-alert) { width: min(310px, 100%); }
.island:not(.call, .reminder) { padding-right: 29px; }
.island.market-alert { padding-right: 81px; }
.market-card { display: flex; align-items: center; gap: 8px; min-width: 0; width: 100%; padding: 0 4px 0 16px; }
.market-instrument { flex: 1; min-width: 0; }
.market-instrument :is(strong, small) { display: block; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.market-instrument strong { font-size: 13px; }
.market-instrument small { font-size: 10px; color: #bfc4c5; }
.market-result { min-width: 0; }
.market-result :is(strong, small) { display: block; }
.market-result small { font-size: 10px; color: #bfc4c5; white-space: nowrap; }
.market-move { flex: 0 1 auto; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-size: 14px; font-variant-numeric: tabular-nums; }
.market-move.up { color: #70df98; }
.market-move.down { color: #ff878a; }
.history-hit {
  position: absolute;
  inset: 0;
  z-index: 3;
  padding: 0;
  border: 0;
  border-radius: inherit;
  background: transparent;
  cursor: grab;
}
.history-hit:active { cursor: grabbing; }
.history-hit:focus-visible { outline: 2px solid #9bc8ff; outline-offset: -3px; }
.island-close, .island :deep(.notification-action) { position: relative; z-index: 4; }
.island-close {
  position: absolute;
  inset: 0 7px 0 auto;
  margin-block: auto;
  width: 23px;
  height: 23px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: #252527;
  color: #ddd;
  font-size: 17px;
  line-height: 1;
  cursor: pointer;
}
.island-close:hover { background: #454549; color: white; }
.island-mute, .island-delete { display: grid; place-items: center; font-size: 14px; }
.island-mute { right: 33px; }
.island-delete { right: 59px; }
.island-delete:hover { background: #5a272a; color: #ffd1d2; }
.island-delete:disabled { opacity: .5; cursor: default; }
.island-mute[aria-pressed="true"] { background: #393a3d; color: #fff; }
.display-error { position: absolute; z-index: 5; left: 16px; bottom: 1px; color: #ff9090; font-size: 10px; max-width: 85%; overflow: hidden; white-space: nowrap; pointer-events: none; }
</style>
