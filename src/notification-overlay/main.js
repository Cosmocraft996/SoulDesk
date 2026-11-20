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
  let renderError;
  const app = createApp({
    render: () => {
      const current = snapshot.value;
      return h('div', { class: 'surface' }, current
        ? [h(Island, { host, snapshot: current, key: current.notification.id })]
        : []);
    },
  });
  app.config.errorHandler = error => { renderError = error; };
  const stopContent = await listen('host:island-content', async ({ payload }) => {
    renderError = undefined;
    snapshot.value = payload;
    const id = payload?.notification.id;
    if (!id) return;
    await nextTick();
    if (snapshot.value?.notification.id !== id) return;
    try {
      if (renderError) throw renderError;
      await command({ type: 'painted', id });
    } catch (error) {
      await command({ type: 'failed', id, message: String(error) }).catch(() => undefined);
    }
  });
  const stopShown = await listen('host:island-shown', ({ payload }) => {
    const current = snapshot.value;
    if (current?.notification.id === payload) snapshot.value = { ...current, displayed: true };
  });
  app.mount('#root');
  window.addEventListener('pagehide', () => {
    stopContent();
    stopShown();
    app.unmount();
  }, { once: true });
  await command({ type: 'ready' });
}

void mountNotificationOverlay().catch(error =>
  command({ type: 'failed', message: String(error) }).catch(() => undefined));
