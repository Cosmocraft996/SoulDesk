import { createApp, h, nextTick, shallowRef } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '../host';
import Island from './Island.vue';
import license from './LICENSE.txt?url';
import './style.css';

const command = args => invoke('island_surface', { command: args });
const host = {
  preview: false,
  request: (method, args) => command({ ...args, type: method.slice('island.'.length) }),
};

async function mountNotificationOverlay() {
  const licenseLink = document.createElement('link');
  licenseLink.rel = 'license';
  licenseLink.href = license;
  document.head.append(licenseLink);
  const snapshot = shallowRef(null);
  const audible = shallowRef(null);
  const marketAudioError = shallowRef(null);
  const marketAudio = new Audio();
  marketAudio.loop = true;
  let marketId = null, marketTone = null, marketUrl, marketSequence = 0;
  const pendingError = shallowRef('');
  let renderError;
  function stopMarketSound() {
    marketSequence++;
    marketId = null;
    marketTone = null;
    marketAudio.pause();
    marketAudio.removeAttribute('src');
    if (marketUrl) URL.revokeObjectURL(marketUrl);
    marketUrl = undefined;
    marketAudioError.value = null;
  }
  async function playMarketSound(id, tone) {
    if (id === marketId && tone === marketTone) return;
    stopMarketSound();
    if (!id || !tone) return;
    marketId = id;
    marketTone = tone;
    const sequence = marketSequence;
    try {
      const encoded = await command({ type: 'market-sound', id, tone });
      if (marketSequence !== sequence) return;
      const bytes = Uint8Array.from(atob(encoded), char => char.charCodeAt(0));
      marketUrl = URL.createObjectURL(new Blob([bytes], { type: 'audio/mpeg' }));
      marketAudio.src = marketUrl;
      await marketAudio.play();
      if (marketSequence === sequence) await command({ type: 'market-sound-started', id, tone });
    } catch (error) {
      if (marketSequence === sequence) marketAudioError.value = { id, message: String(error) };
    }
  }
  const app = createApp({
    render: () => {
      const current = snapshot.value;
      return h('div', { class: 'surface', style: { gap: `${current?.gap ?? 0}px`, opacity: (current?.opacity ?? 100) / 100 } }, current
        ? [
            ...current.items.map(item => h(Island, {
              key: item.notification.id, host, notification: item.notification,
              height: item.height, sampledAt: current.sampledAt,
              audible: audible.value === item.notification.id,
              audioMuted: current.muted === true,
              audioVolume: (current.volume ?? 100) / 100,
              audioError: marketAudioError.value?.id === item.notification.id ? marketAudioError.value.message : '',
            })),
            current.pendingCount && h('button', {
              type: 'button', class: 'island-more', style: { height: `${current.moreHeight}px` },
              title: pendingError.value || current.next?.message,
              onClick: () => command({ type: 'pending' }).catch(error => { pendingError.value = String(error); }),
            }, [h('strong', `更多 ${current.pendingCount} 条`), h('span', pendingError.value || current.next?.title || current.next?.message)]),
          ]
        : []);
    },
  });
  app.config.errorHandler = error => { renderError = error; };
  const stopContent = await listen('host:island-content', async ({ payload }) => {
    renderError = undefined;
    pendingError.value = '';
    snapshot.value = payload;
    marketAudio.muted = payload?.muted === true || payload?.marketAudioMuted === true;
    marketAudio.volume = (payload?.volume ?? 100) / 100;
    const id = payload?.id;
    if (marketId && (payload?.audibleMarket !== marketId || payload?.audibleMarketTone !== marketTone)) stopMarketSound();
    if (!id) { audible.value = null; return; }
    await nextTick();
    if (snapshot.value?.id !== id) return;
    try {
      if (renderError) throw renderError;
      await command({ type: 'painted', id });
    } catch (error) {
      await command({ type: 'failed', id, message: String(error) }).catch(() => undefined);
    }
  });
  const stopShown = await listen('host:island-shown', ({ payload }) => {
    const current = snapshot.value;
    if (current?.id === payload) {
      audible.value = current.audible ?? null;
      void playMarketSound(current.audibleMarket ?? null, current.audibleMarketTone ?? null);
    }
  });
  app.mount('#root');
  window.addEventListener('pagehide', () => {
    stopMarketSound();
    stopContent();
    stopShown();
    app.unmount();
  }, { once: true });
  await command({ type: 'ready' });
}

void mountNotificationOverlay().catch(error =>
  command({ type: 'failed', message: String(error) }).catch(() => undefined));
