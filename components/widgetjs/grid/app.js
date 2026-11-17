import { connectHost } from './host.js';
import { visibleFiles, fileLabel, formatGroups, toggleFormats, validateSettings } from './grid-model.js';
const $ = id => document.getElementById(id), host = await connectHost();
let settings = await host.request('settings.get'), state = { revision: null, root: null, files: [], excluded: [] }, editing = false, generation = 0, disposed = false, pending = 0, queue = [], observer, lastOpen = 0, operation = 0;
const iconCache = new Map();
for (const event of ['dragover', 'drop']) {
    document.querySelector('.content').addEventListener(event, event => event.preventDefault());
}
function message(value) { $('status').textContent = $('config-status').textContent = String(value?.message || value || ''); }
function geometry() { const cell = Math.floor(document.documentElement.clientWidth / settings.gridCount); document.documentElement.style.setProperty('--cell', cell + 'px'); document.documentElement.style.setProperty('--icon', Math.max(8, cell - 24) + 'px'); }
function theme() { for (const [key, value] of Object.entries({ background: settings.background, color: settings.color, radius: settings.radius + 'px', columns: settings.gridCount }))
    document.documentElement.style.setProperty('--' + key, value); $('title').textContent = settings.title; geometry(); }
function fillForm() { for (const [id, value] of Object.entries({ name: settings.title, columns: settings.gridCount, include: settings.includeFormats, exclude: settings.excludeFormats, background: settings.background, color: settings.color, radius: settings.radius, 'sort-by': settings.sortBy, 'sort-mode': settings.sortMode }))
    $(id).value = value; updateFormatGroups(); }
function updateFormatGroups() { const current = $('include').value.split(/\s+/); for (const input of $('formats').querySelectorAll('input'))
    input.checked = input.dataset.formats.split(' ').every(format => current.includes(format)); }
for (const [icon, label, formats] of formatGroups) {
    const wrapper = document.createElement('label'), image = document.createElement('img'), input = document.createElement('input'), text = document.createElement('span');
    image.src = './' + icon;
    image.alt = '';
    input.type = 'checkbox';
    input.dataset.formats = formats;
    text.textContent = label;
    input.onchange = () => { $('include').value = toggleFormats($('include').value, formats, input.checked); updateFormatGroups(); };
    wrapper.append(image, text, input);
    $('formats').append(wrapper);
}
$('include').oninput = updateFormatGroups;
function iconUrl(value) { const bytes = Uint8ClampedArray.from(atob(value.rgba), char => char.charCodeAt(0)); if (!Number.isInteger(value.width) || !Number.isInteger(value.height) || value.width < 1 || value.height < 1 || value.width > 96 || value.height > 96 || bytes.length !== value.width * value.height * 4)
    throw new Error('系统图标数据异常'); const canvas = document.createElement('canvas'); canvas.width = value.width; canvas.height = value.height; canvas.getContext('2d').putImageData(new ImageData(bytes, value.width, value.height), 0, 0); return canvas.toDataURL('image/png'); }
function pump() { while (pending < 4 && queue.length) {
    const task = queue.shift();
    if (task.generation !== generation || disposed)
        continue;
    pending++;
    void (async () => { try {
        const key = task.file.displayPath + ':' + task.file.modifiedTime + ':' + task.file.size;
        let url = iconCache.get(key);
        if (!url) {
            url = iconUrl(await host.request('files.icon', { id: task.file.id, revision: task.revision }));
            if (task.generation !== generation || disposed)
                return;
            iconCache.set(key, url);
            if (iconCache.size > 1024)
                iconCache.delete(iconCache.keys().next().value);
        }
        if (task.generation === generation && !disposed)
            task.image.src = url;
    }
    catch (error) {
        if (task.generation === generation && !disposed) {
            task.image.src = './file_fail.ico';
            task.image.title = '图标未加载：' + String(error?.message || error);
        }
    }
    finally {
        pending--;
        pump();
    } })();
} }
async function openFile(file, button) { if (host.preview || editing || button?.classList.contains('opening') || Date.now() - lastOpen < 500)
    return; lastOpen = Date.now(); button?.classList.add('opening'); try {
    await host.request('files.open', { id: file.id, revision: state.revision });
    message(state.rootError || '');
}
catch (error) {
    message(error);
}
finally {
    button?.classList.remove('opening');
} }
function render() { generation++; queue = []; observer?.disconnect(); observer = new IntersectionObserver(entries => { for (const entry of entries) {
    if (!entry.isIntersecting)
        continue;
    observer.unobserve(entry.target);
    const task = entry.target.iconTask;
    if (task) {
        queue.push(task);
        pump();
    }
} }, { root: $('scroll'), rootMargin: '60px' }); const displayed = visibleFiles(state.files, settings), fragment = document.createDocumentFragment(); for (const file of displayed) {
    const item = document.createElement('div'), button = document.createElement('button'), image = document.createElement('img'), label = document.createElement('span'), remove = document.createElement('button'), removeImage = document.createElement('img');
    item.className = 'file' + (file.unavailable ? ' unavailable' : '');
    button.type = 'button';
    button.className = 'file-open';
    button.setAttribute('aria-label', '打开 ' + file.name);
    button.title = file.displayPath + (file.error ? '\n' + file.error : '');
    image.className = 'icon';
    image.src = './file.ico';
    image.alt = '';
    image.draggable = false;
    label.className = 'label';
    label.textContent = fileLabel(file);
    button.append(image, label);
    button.onclick = () => void openFile(file, item);
    remove.type = 'button';
    remove.className = 'remove';
    remove.hidden = !editing;
    remove.title = '排除 ' + file.name + '（保留原文件）';
    remove.setAttribute('aria-label', remove.title);
    removeImage.src = './delete.png';
    removeImage.alt = '';
    remove.append(removeImage);
    remove.onclick = () => void change('files.exclude', { id: file.id });
    item.append(button, remove);
    if (!file.unavailable) {
        item.iconTask = { file, image, revision: state.revision, generation };
        observer.observe(item);
    }
    fragment.append(item);
} $('grid').replaceChildren(fragment); $('empty').hidden = Boolean(displayed.length); $('empty').textContent = host.preview ? '预览 · 添加后选择文件或主目录' : state.rootError ? '主目录未能读取，可在设置中重新选择' : '右键添加文件或文件夹'; $('folder').value = state.root?.displayPath || ''; $('clear-root').disabled = host.preview || !state.root; $('restore-all').disabled = host.preview || !state.excluded.length; $('excluded').replaceChildren(); for (const entry of state.excluded) {
    const row = document.createElement('div'), text = document.createElement('span'), restore = document.createElement('button');
    row.className = 'excluded-row';
    text.textContent = entry.displayPath;
    restore.type = 'button';
    restore.textContent = '恢复显示';
    restore.onclick = () => void change('files.restore', { id: entry.id });
    row.append(text, restore);
    $('excluded').append(row);
} $('edit').setAttribute('aria-pressed', String(editing)); }
function apply(value) { state = value; render(); message(state.rootError || ''); }
async function refresh() { if (host.preview || disposed)
    return; const version = ++operation; try {
    const value = await host.request('files.list');
    if (version === operation && !disposed)
        apply(value);
}
catch (error) {
    if (version === operation && !disposed)
        message(error);
} }
function busy(value) { for (const button of document.querySelectorAll('dialog button,menu button'))
    button.disabled = value || host.preview; if (!value) {
    $('clear-root').disabled = host.preview || !state.root;
    $('restore-all').disabled = host.preview || !state.excluded.length;
} }
async function change(method, args = {}) { if (host.preview || disposed)
    return; const version = ++operation; busy(true); try {
    const value = await host.request(method, { ...args, revision: state.revision });
    if (value && version === operation && !disposed)
        apply(value);
}
catch (error) {
    if (version === operation && !disposed)
        message(error);
}
finally {
    busy(false);
} }
const choose = mode => change('documents.open', { launcher: mode });
$('title').onclick = () => { if (state.root)
    void openFile(state.root); };
$('add-file').onclick = () => choose('file');
$('add-folder').onclick = () => choose('folder');
$('choose-root').onclick = $('folder').onclick = () => choose('root');
$('clear-root').onclick = () => change('files.clear');
$('restore-all').onclick = () => change('files.restore');
$('refresh').onclick = refresh;
$('refresh-icons').onclick = () => { iconCache.clear(); void refresh(); };
$('edit').onclick = () => { editing = !editing; render(); };
$('settings').onclick = () => { fillForm(); $('config').showModal(); };
$('close').onclick = () => $('config').close();
$('sort').onclick = () => { fillForm(); $('sort-dialog').showModal(); };
$('sort-close').onclick = () => $('sort-dialog').close();
async function save(next, status) { validateSettings(next); const saved = await host.request('settings.update', next); settings = saved || next; theme(); render(); $(status).textContent = '已保存'; }
$('form').onsubmit = async (event) => { event.preventDefault(); busy(true); try {
    await save({ ...settings, title: $('name').value, gridCount: Number($('columns').value), includeFormats: $('include').value, excludeFormats: $('exclude').value, background: $('background').value, color: $('color').value, radius: Number($('radius').value) }, 'config-status');
}
catch (error) {
    $('config-status').textContent = '未保存：' + String(error?.message || error);
}
finally {
    busy(false);
} };
$('sort-form').onsubmit = async (event) => { event.preventDefault(); busy(true); try {
    await save({ ...settings, sortBy: $('sort-by').value, sortMode: $('sort-mode').value }, 'sort-status');
    $('sort-dialog').close();
}
catch (error) {
    $('sort-status').textContent = '未保存：' + String(error?.message || error);
}
finally {
    busy(false);
} };
document.oncontextmenu = event => { if (host.preview)
    return; event.preventDefault(); const menu = $('context'); menu.hidden = false; menu.style.left = Math.max(0, Math.min(event.clientX, innerWidth - menu.offsetWidth)) + 'px'; menu.style.top = Math.max(0, Math.min(event.clientY, innerHeight - menu.offsetHeight)) + 'px'; };
document.addEventListener('click', () => { $('context').hidden = true; });
document.addEventListener('keydown', event => { if (event.key === 'Escape' || event.key === 'Enter') {
    if (editing) {
        editing = false;
        render();
    }
    $('context').hidden = true;
} });
const resize = new ResizeObserver(geometry);
resize.observe(document.documentElement);
window.addEventListener('pagehide', () => { disposed = true; operation++; generation++; queue = []; observer?.disconnect(); resize.disconnect(); iconCache.clear(); }, { once: true });
theme();
fillForm();
render();
if (host.preview)
    busy(true);
await host.ready();
if (!host.preview)
    void refresh();
