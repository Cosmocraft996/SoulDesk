// 保留独立函数，验证相对模块和 CSS 最终解析到同一组件目录。
export function widgetEntryUrl(convertedUrl: string): URL {
  const url = new URL(convertedUrl);
  // convertFileSrc 把斜线也编码了；展开目录后浏览器才能解析相对资源。
  url.pathname = decodeURIComponent(url.pathname);
  return url;
}
