import test from 'node:test';
import assert from 'node:assert/strict';
import { visibleFiles, toggleFormats, fileLabel, validateSettings } from './grid-model.js';
const settings = { title: '文件夹', gridCount: 5, sortBy: 'name', sortMode: 'asc', includeFormats: '', excludeFormats: '', background: '#061d2e', color: '#ffffff', radius: 24 };
test('按原规则先文件夹，名称忽略大小写，格式过滤保留大小写且排除优先', () => {
    const files = [{ name: 'z.bin', isDirectory: true }, { name: 'B.pdf' }, { name: 'a.PDF' }, { name: 'c.doc' }, { name: 'a.pdf' }];
    assert.deepEqual(visibleFiles(files, { ...settings, includeFormats: '.pdf .PDF', excludeFormats: '.PDF' }).map(file => file.name), ['z.bin', 'a.pdf', 'B.pdf']);
    assert.deepEqual(visibleFiles(files, { ...settings, sortMode: 'desc' }).map(file => file.name), ['z.bin', 'c.doc', 'B.pdf', 'a.PDF', 'a.pdf']);
    assert.equal(files[0].name, 'z.bin');
});
test('按实际创建或修改时间排序，缺少时间的项目保留未知且排在最后', () => {
    const files = [{ name: 'a', modifiedTime: 100 }, { name: 'b', modifiedTime: null }, { name: 'c', modifiedTime: 200 }];
    assert.deepEqual(visibleFiles(files, { ...settings, sortBy: 'modifiedTime', sortMode: 'desc' }).map(file => file.name), ['c', 'a', 'b']);
    assert.equal(files[1].modifiedTime, null);
});
test('格式组增删不重复，多点文件名和带点文件夹名称保持可辨识', () => {
    assert.equal(toggleFormats('.pdf .doc', '.doc .docx', true), '.pdf .doc .docx');
    assert.equal(toggleFormats('.pdf .doc .docx', '.doc .docx', false), '.pdf');
    assert.equal(fileLabel({ name: '年度.修订.pdf' }), '年度.修订');
    assert.equal(fileLabel({ name: '资料.v2', isDirectory: true }), '资料.v2');
    assert.equal(fileLabel({ name: '.gitignore' }), '.gitignore');
    assert.throws(() => validateSettings({ ...settings, gridCount: 3 }));
    assert.throws(() => validateSettings({ ...settings, color: 'url(x)' }));
});
