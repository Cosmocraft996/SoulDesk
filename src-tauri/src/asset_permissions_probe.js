// 仅显式隔离验收装入的测试包使用，逐项核对真实WebView的允许和拒绝行为。
import { connectHost } from './host.js';
const host = await connectHost();
let stage = '初始化';
window.addEventListener('error', event => {
  if (!event.message && !(event.target instanceof HTMLImageElement)) {
    parent.postMessage({type:'souldesk:failed',protocol:1,message:`资源探针${stage}：${event.target?.tagName || '未知资源'}加载失败`}, new URLSearchParams(location.search).get('hostOrigin'));
  }
}, true);
const capabilities = new Set(PROBE_CAPABILITIES);
const legacy = capabilities.has('assets.local');
const has = name => legacy || capabilities.has(name);
const equal = (name, actual, expected) => {
  if (actual !== expected) throw new Error(`${name}：实际${actual}，预期${expected}`);
};
const canEval = () => { try { return Function('return 7')() === 7; } catch { return false; } };
const canWasm = async () => {
  try { await WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])); return true; } catch { return false; }
};
equal('动态代码', canEval(), has('runtime.eval'));
equal('WASM', await canWasm(), has('runtime.wasm') || has('runtime.eval'));

const element = document.createElement('div');
element.className = 'asset-probe';
document.body.append(element);
stage = '样式属性文本';
// 被拦截的style标签会触发正式资源错误监控；拒绝行为用样式属性文本核对。
element.setAttribute('style', 'color:rgb(1,2,3)');
equal('动态样式', getComputedStyle(element).color === 'rgb(1, 2, 3)', has('styles.inline'));
element.removeAttribute('style');
if (has('styles.inline')) {
  const style = document.createElement('style');
  style.textContent = '.asset-probe {color:rgb(1,2,3)}';
  document.head.append(style);
  equal('动态样式标签', getComputedStyle(element).color === 'rgb(1, 2, 3)', true);
}
element.style.width = '17px';
equal('普通样式属性赋值', getComputedStyle(element).width === '17px', true);

stage = 'Blob图片';
const picture = new Image();
const pictureUrl = URL.createObjectURL(new Blob(['<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>'], {type:'image/svg+xml'}));
const loaded = await new Promise(resolve => {
  picture.onload = () => resolve(true);
  picture.onerror = event => { event.preventDefault(); resolve(false); };
  picture.src = pictureUrl;
});
URL.revokeObjectURL(pictureUrl);
equal('Blob图片', loaded, has('images.blob'));
let fetched = false;
stage = '同源读取';
try { fetched = (await (await fetch('./probe.json')).json()).ok === true; } catch { /* 预期的权限拒绝 */ }
equal('同来源读取', fetched, has('assets.fetch'));

const workerSource = `let evaluated=false;try{evaluated=Function('return 7')()===7}catch{};WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])).then(()=>postMessage({evaluated,wasm:true}),()=>postMessage({evaluated,wasm:false}));`;
const worker = async (url, expected, label) => {
  let instance, timer;
  const result = await new Promise(resolve => {
    try {
      instance = new Worker(url);
      instance.onmessage = event => resolve(event.data);
      instance.onerror = event => { event.preventDefault(); resolve(null); };
      timer = setTimeout(() => resolve('timeout'), 2000);
    } catch { resolve(null); }
  });
  clearTimeout(timer);
  instance?.terminate();
  if (result === 'timeout') throw new Error(`${label}超时`);
  equal(label, result !== null, expected);
  if (expected) {
    equal(`${label}动态代码`, result.evaluated, has('runtime.eval'));
    equal(`${label}WASM`, result.wasm, has('runtime.wasm') || has('runtime.eval'));
  }
};
stage = '本地线程';
await worker('./worker.js', has('workers.local'), '本地线程');
stage = 'Blob线程';
const blobUrl = URL.createObjectURL(new Blob([workerSource], {type:'text/javascript'}));
try { await worker(blobUrl, has('workers.blob'), 'Blob线程'); } finally { URL.revokeObjectURL(blobUrl); }
await host.ready();
