const imageFormats = ['png','jpg','jpeg','svg','gif','webp','avif','bmp','ico','heic','heif'];
const allowed = ['excalidraw','excalidrawlib','json',...imageFormats,'txt','md','yaml','yml','zip'];
function abort() { return new DOMException('用户取消文件选择或当前为预览', 'AbortError'); }
function optionsFor(input = {}) {
  const options = Array.isArray(input) ? input[0] || {} : input;
  let extensions = options.extensions?.map(value => String(value).replace(/^\./,'').toLowerCase());
  if (!extensions?.length) {
    const mime = options.mimeTypes || [];
    extensions = mime.includes('image/*') ? [...imageFormats] : mime.flatMap(type => type.startsWith('image/') ? imageFormats.filter(extension => type === 'image/' + (extension === 'jpg' ? 'jpeg' : extension === 'svg' ? 'svg+xml' : extension)) : type.includes('excalidrawlib') ? ['excalidrawlib'] : type.includes('excalidraw') ? ['excalidraw'] : type.includes('json') ? ['json'] : []);
  }
  if (!extensions.length) extensions = ['excalidraw','json','png','svg'];
  if (extensions.some(value => !allowed.includes(value))) throw new Error('不支持的文档格式');
  extensions = [...new Set(extensions)];
  let fileName = String(options.fileName || '未命名');
  if (!extensions.some(extension => fileName.toLowerCase().endsWith('.' + extension))) fileName += '.' + extensions[0];
  if (/[\\/:<>"|?*]/.test(fileName) || fileName.startsWith('.')) throw new Error('文件名不能包含路径');
  return {fileName,extensions,multiple:options.multiple === true};
}
function base64(bytes) {
  let text = '';
  for (let offset = 0; offset < bytes.length; offset += 16384) text += String.fromCharCode(...bytes.subarray(offset, offset + 16384));
  return btoa(text);
}
export function createFileAccess(host) {
  function requireHost() { if (!host || host.preview) throw abort(); }
  return {
    async fileOpen(options) {
      requireHost(); const args = optionsFor(options);
      const files = await host.request('documents.open', args);
      if (!files?.length) throw abort();
      const result = files.map(file => {
        const decoded = atob(file.data), bytes = Uint8Array.from(decoded, value => value.charCodeAt(0));
        return new File([bytes], file.name, {type:file.mime});
      });
      return args.multiple ? result : result[0];
    },
    async fileSave(blobOrPromise, options) {
      requireHost(); const blob = await (typeof blobOrPromise === 'function' ? blobOrPromise() : blobOrPromise);
      if (!(blob instanceof Blob)) throw new Error('导出内容不是有效文件');
      if (blob.size > 32 * 1024 * 1024) throw new Error('导出文件超过32 MiB，请缩小图片后再保存');
      const args = optionsFor(options);
      const result = await host.request('documents.save', {...args,data:base64(new Uint8Array(await blob.arrayBuffer()))});
      if (!result) throw abort();
      // 每次保存均经用户选择，沿用browser-fs-access的无持久文件句柄分支。
      return null;
    },
  };
}
let current;
export function setFileHost(host) { current = createFileAccess(host); }
export const supported = false;
export const fileOpen = (...args) => current.fileOpen(...args);
export const fileSave = (...args) => current.fileSave(...args);
