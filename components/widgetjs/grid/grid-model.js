// 保留原useFileSort的大小写敏感格式过滤、名称忽略大小写和文件夹优先。
export const formatGroups = [
    ['word.png', 'Word', '.doc .docx'], ['excel.png', 'Excel', '.xls .xlsx'],
    ['ppt.png', 'PPT', '.ppt'], ['pdf.png', 'PDF', '.pdf'],
    ['image.png', '图片', '.jpg .jpeg .png .gif .bmp .webp .svg .tiff .tif'],
    ['audio.png', '音频', '.mp3 .wav .aac .ogg .flac'], ['video.png', '视频', '.mp4 .avi .mkv .flv .webm .mov'],
];
const formats = text => String(text || '').trim().split(/\s+/).filter(Boolean);
export function toggleFormats(current, group, include) {
    const values = new Set(formats(current));
    for (const format of formats(group)) {
        if (include)
            values.add(format);
        else
            values.delete(format);
    }
    return [...values].join(' ');
}
export function visibleFiles(files, settings) {
    const include = formats(settings.includeFormats), exclude = formats(settings.excludeFormats);
    const compare = (a, b) => {
        const field = settings.sortBy;
        const av = field === 'name' ? a.name.toLowerCase() : a[field];
        const bv = field === 'name' ? b.name.toLowerCase() : b[field];
        if (av == null || bv == null)
            return av == null && bv != null ? 1 : av != null && bv == null ? -1 : 0;
        const result = av < bv ? -1 : av > bv ? 1 : 0;
        return settings.sortMode === 'desc' ? -result : result;
    };
    const filtered = files.filter(file => file.isDirectory ||
        (!exclude.some(format => file.name.endsWith(format)) && (!include.length || include.some(format => file.name.endsWith(format)))));
    return [...filtered.filter(file => file.isDirectory).sort(compare), ...filtered.filter(file => !file.isDirectory).sort(compare)];
}
export function fileLabel(file) {
    const dot = file.name.lastIndexOf('.');
    return file.isDirectory || dot <= 0 ? file.name : file.name.slice(0, dot);
}
export function validateSettings(settings) {
    if (!settings.title.trim() || settings.title.length > 120)
        throw new Error('标题需为1～120个字符');
    if (!Number.isInteger(settings.gridCount) || settings.gridCount < 4 || settings.gridCount > 15)
        throw new Error('网格数量需为4～15');
    if (!['name', 'birthTime', 'modifiedTime'].includes(settings.sortBy) || !['asc', 'desc'].includes(settings.sortMode))
        throw new Error('排序设置无效');
    if (settings.includeFormats.length > 1000 || settings.excludeFormats.length > 1000)
        throw new Error('格式过滤内容过长');
    if (![settings.background, settings.color].every(value => /^#[0-9a-f]{6}$/i.test(value)))
        throw new Error('颜色需为六位十六进制颜色');
    if (!Number.isInteger(settings.radius) || settings.radius < 0 || settings.radius > 80)
        throw new Error('圆角需为0～80');
    return settings;
}
