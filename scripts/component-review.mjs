// 本机组件筛选工具；只读组件资料，草稿和提交结果仅写入 artifacts。
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, existsSync, mkdirSync, renameSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash, randomUUID } from 'node:crypto';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const documentPath = join(root, 'docs/20-组件筛选与手动验证清单.md');
const staticRoot = join(root, 'docs/component-review');
const galleryRoot = resolve(process.env.SOULDESK_WIDGETS_ROOT || join(root, '..', 'SoulDesk-Widgets'));
const decisionNames = { '': '未筛选', keep: '保留', trim: '裁剪', undecided: '待定' };
const resultNames = { untested: '待验证', passed: '通过', partial: '部分通过', failed: '失败' };
const sourceCategories = { S1: '行情与汇率', S2: '实用工具', S3: '热点资讯', S4: '时钟与进度', S5: '笔记与图形', S6: 'AI与剪切板', S7: '笔记与图形', S8: '日历与专注', S9: '系统与服务器', S10: '图片与文件', S11: '图片与文件', S12: 'AI与剪切板', S13: 'AI与剪切板', S14: '提醒与进度', S16: '天气', native: '项目原有' };

export function loadReviewCatalog() {
  const text = readFileSync(documentPath, 'utf8');
  const rows = text.split('\n').filter(line => /^\| \d{2} \|/.test(line));
  const sources = new Map(text.split('\n').filter(line => /^\| S\d+ \|/.test(line)).map(line => {
    const cells = line.split('|').slice(1, -1).map(cell => cell.trim());
    const link = cells[1].match(/\[([^\]]+)\]\(([^)]+)\)/);
    return [cells[0], { label: link[1], pinnedUrl: link[2] }];
  }));
  const upstream = JSON.parse(readFileSync(join(root, 'docs/widgetjs-catalog.json'), 'utf8'));
  const programLink = text.match(/\]\(\.\.\/(artifacts\/[^)]+-program\.json)\)/)?.[1];
  const program = programLink && existsSync(join(root, programLink)) ? JSON.parse(readFileSync(join(root, programLink), 'utf8')) : null;
  const items = rows.map(line => {
    const cells = line.split('|').slice(1, -1).map(cell => cell.trim());
    const number = Number(cells[0]), slug = cells[1].match(/`([^`]+)`/)[1];
    const name = cells[1].match(/\[([^\]]+)\]/)?.[1] || cells[1].split('`')[0].trim();
    const id = number <= 61 ? `local.souldesk.widgetjs.${slug}` : slug;
    const gallerySlug = number <= 61 ? slug : 'market';
    const manifestPath = join(galleryRoot, 'widgets', gallerySlug, 'manifest.json');
    const manifest = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, 'utf8')) : { version: '历史记录' };
    const upstreamItem = upstream.items.find(item => item.souldeskId === id);
    const source = sources.get(cells[3]) || { label: 'SoulDesk 项目', pinnedUrl: 'https://github.com/Cosmocraft996/SoulDesk' };
    const projectUrl = source.pinnedUrl.replace(/\/tree\/[^/]+.*$/, '');
    return { number, id, slug, name, version: manifest.version,
      summary: cells[2].replace(/†/g, '').trim(), cloudDeferred: cells[2].includes('†'),
      initialDecision: cells[4] === '☑' ? 'keep' : cells[5] === '☑' ? 'trim' : cells[6] === '☑' ? 'undecided' : '',
      initialResult: ({ '待验证': 'untested', '通过': 'passed', '部分通过': 'partial', '失败': 'failed' })[cells[7]] || 'untested',
      category: sourceCategories[cells[3]] || sourceCategories.native, sourceLabel: source.label, projectUrl,
      pinnedUrl: source.pinnedUrl, originalTitle: upstreamItem?.title || name,
      originalDescription: upstreamItem?.description || '',
      sourcePath: number <= 61 ? join(galleryRoot, 'widgets', gallerySlug, 'src/SOURCE.txt') : manifestPath };
  });
  if (new Set(items.map(item => item.id)).size !== items.length || items.length === 0) throw Error('清单存在重复标识或没有组件');
  const build = {
    version: program?.version || JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version,
    sha256: program?.sha256 || '',
    label: program?.sha256.slice(0, 8) || '当前源码',
  };
  return { schemaVersion: 1, id: createHash('sha256').update(JSON.stringify({ items, build })).digest('hex'), build,
    sourceVersion: JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version,
    items };
}

export function createReviewServer({ stateDir = join(root, 'artifacts/component-review'), catalog = loadReviewCatalog() } = {}) {
  const draftPath = join(stateDir, 'draft.json');
  const initialEntries = (items = catalog.items) => Object.fromEntries(items.map(item => [item.id, { decision: item.initialDecision || '', result: item.initialResult || 'untested', notes: '', testedVersion: item.version }]));
  function current() {
    const stored = existsSync(draftPath) ? JSON.parse(readFileSync(draftPath, 'utf8')) : { revision: 0, updatedAt: null };
    const entries = initialEntries();
    for (const id of Object.keys(entries)) if (stored.entries?.[id]) entries[id] = { ...entries[id], ...stored.entries[id] };
    return { schemaVersion: 1, revision: stored.revision || 0, updatedAt: stored.updatedAt || null, catalogId: catalog.id, entries, catalogChanged: !!stored.catalogId && stored.catalogId !== catalog.id };
  }
  function validate(body) {
    if (body.catalogId !== catalog.id) throw Object.assign(Error('组件目录已更新，请刷新页面后继续'), { status: 409 });
    if (!body.entries || Object.keys(body.entries).length !== catalog.items.length) throw Error(`结果必须包含当前 ${catalog.items.length} 项组件`);
    const entries = {};
    for (const item of catalog.items) {
      const value = body.entries[item.id];
      if (!value || !Object.hasOwn(decisionNames, value.decision) || !Object.hasOwn(resultNames, value.result)) throw Error('筛选或验证状态无效');
      if (typeof value.notes !== 'string' || value.notes.length > 8000 || typeof value.testedVersion !== 'string' || value.testedVersion.length > 120) throw Error('备注或版本格式无效');
      entries[item.id] = { decision: value.decision, result: value.result, notes: value.notes, testedVersion: value.testedVersion };
    }
    return entries;
  }
  function save(body) {
    const entries = validate(body), previous = current();
    if (body.revision !== previous.revision) throw Object.assign(Error('另一页面已保存了更新的草稿；当前输入仍保留在浏览器，请导出后刷新核对'), { status: 409 });
    const state = { schemaVersion: 1, catalogId: catalog.id, revision: previous.revision + 1, updatedAt: new Date().toISOString(), entries };
    mkdirSync(stateDir, { recursive: true });
    writeFileSync(draftPath + '.pending', JSON.stringify(state, null, 2) + '\n');
    renameSync(draftPath + '.pending', draftPath);
    return state;
  }
  const json = (res, status, value) => { res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' }); res.end(JSON.stringify(value)); };
  const server = createServer(async (req, res) => {
    try {
      const host = req.headers.host;
      if (!/^(127\.0\.0\.1|localhost):\d+$/.test(host || '')) return json(res, 403, { error: '仅供本机访问' });
      if (req.headers.origin && req.headers.origin !== `http://${host}`) return json(res, 403, { error: '不接受其他网站的请求' });
      const url = new URL(req.url, `http://${host}`);
      if (req.method === 'GET' && url.pathname === '/api/health') return json(res, 200, { service: 'souldesk-component-review', items: catalog.items.length });
      if (req.method === 'GET' && url.pathname === '/api/catalog') return json(res, 200, catalog);
      if (req.method === 'GET' && url.pathname === '/api/state') return json(res, 200, current());
      if (req.method === 'GET' && url.pathname === '/api/source') {
        const item = catalog.items.find(item => item.id === url.searchParams.get('id'));
        if (!item) return json(res, 404, { error: '组件不存在' });
        const contents = readFileSync(item.sourcePath);
        res.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8', 'X-Content-Type-Options': 'nosniff' });
        return res.end(contents);
      }
      if (['PUT', 'POST'].includes(req.method) && ['/api/state', '/api/submit'].includes(url.pathname)) {
        if (!req.headers['content-type']?.startsWith('application/json')) return json(res, 415, { error: '需要 JSON 格式' });
        const chunks = []; let bytes = 0;
        for await (const chunk of req) { bytes += chunk.length; if (bytes > 2 * 1024 * 1024) throw Error('内容过大'); chunks.push(chunk); }
        // 分块在完整字节串上解码，避免中文字符被网络分片截断。
        const input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
        if (url.pathname === '/api/state') return json(res, 200, save(input));
        const entries = validate(input), state = current();
        if (input.revision !== state.revision || JSON.stringify(entries) !== JSON.stringify(state.entries)) throw Object.assign(Error('草稿已变化，请先保存当前输入再提交'), { status: 409 });
        const submittedAt = new Date().toISOString(), name = `review-${submittedAt.replace(/[:.]/g, '-')}-${randomUUID().slice(0, 8)}`;
        const directory = join(stateDir, 'submissions'); mkdirSync(directory, { recursive: true });
        const report = { schemaVersion: 1, submittedAt, build: catalog.build, catalogId: catalog.id,
          items: catalog.items.map(item => ({ ...item, ...state.entries[item.id] })) };
        const clean = value => String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\|/g, '\\|').replace(/\r?\n/g, '<br>');
        const markdown = ['# 组件筛选与验证结果', '', `提交时间：${submittedAt}。验收构建：${catalog.build.version}（${catalog.build.label}）。`, '',
          '| 编号 | 组件 | 筛选 | 验证结果 | 实测版本 | 备注 | 原项目 |', '| --- | --- | --- | --- | --- | --- | --- |',
          ...report.items.map(item => `| ${String(item.number).padStart(2, '0')} | ${clean(item.name)} | ${decisionNames[item.decision]} | ${resultNames[item.result]} | ${clean(item.testedVersion)} | ${clean(item.notes)} | [原项目](${item.projectUrl}) |`), '',
          ''].join('\n');
        writeFileSync(join(directory, name + '.json'), JSON.stringify(report, null, 2) + '\n', { flag: 'wx' });
        writeFileSync(join(directory, name + '.md'), markdown, { flag: 'wx' });
        const receipt = { submittedAt, files: [join(directory, name + '.json'), join(directory, name + '.md')], revision: state.revision };
        writeFileSync(join(stateDir, 'latest-submission.json.pending'), JSON.stringify(receipt, null, 2) + '\n');
        renameSync(join(stateDir, 'latest-submission.json.pending'), join(stateDir, 'latest-submission.json'));
        return json(res, 201, receipt);
      }
      const assets = { '/': ['index.html', 'text/html'], '/app.js': ['app.js', 'text/javascript'], '/style.css': ['style.css', 'text/css'] };
      const asset = req.method === 'GET' && assets[url.pathname];
      if (!asset) return json(res, 404, { error: '页面不存在' });
      const contents = readFileSync(join(staticRoot, asset[0]));
      res.writeHead(200, { 'Content-Type': asset[1] + '; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff',
        'Content-Security-Policy': "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'" });
      res.end(contents);
    } catch (error) { json(res, error.status || 400, { error: error.message }); }
  });
  return server;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const port = Number(process.argv[2] || 43187);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw Error('端口无效');
  const server = createReviewServer();
  server.on('error', error => { console.error(`组件验收网页启动失败：${error.message}`); process.exitCode = 1; });
  server.listen(port, '127.0.0.1', () => console.log(`组件验收网页：http://127.0.0.1:${port}\n草稿及提交：${join(root, 'artifacts/component-review')}`));
}
