import test from 'node:test';
import assert from 'node:assert/strict';
import { fallbackRemoteImage } from './images.js';
test('外部图片失败切换占位并留出宿主恢复机会，包内和占位错误继续交给宿主', () => {
  const previous = globalThis.HTMLImageElement;
  class Image { constructor(src) { this.src = src; this.srcset = ''; this.dataset = {}; } removeAttribute(key) { this[key] = ''; } }
  globalThis.HTMLImageElement = Image;
  try {
    const status = { failed: 0 }, image = new Image('https://example.org/cover.png');
    image.srcset = 'https://example.org/cover@2.png 2x';
    assert.equal(fallbackRemoteImage({ target: image }, status), true);
    assert.match(image.src, /^data:image\/svg\+xml;base64,/); assert.equal(image.srcset, '');
    assert.equal(image.dataset.failedRemote, '1'); assert.equal(status.failed, 1);
    assert.equal(fallbackRemoteImage({ target: image }, status), false);
    const local = new Image('http://widget.localhost/package/logo.png');
    assert.equal(fallbackRemoteImage({ target: local }, status), false); assert.equal(status.failed, 1);
    assert.equal(fallbackRemoteImage({ target: {} }, status), false);
  } finally { if (previous === undefined) delete globalThis.HTMLImageElement; else globalThis.HTMLImageElement = previous; }
});
