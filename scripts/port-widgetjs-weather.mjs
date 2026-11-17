import { readFileSync, writeFileSync, copyFileSync, mkdirSync, readdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';

const upstream = 'artifacts/widgetjs-upstream/weather';
const revision = '1d5b717b085c82a5b81bfc2f0969bb8518123570';
const result = spawnSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding: 'utf8' });
if (result.status !== 0 || result.stdout.trim() !== revision) throw Error('天气来源提交不符');
const target = 'components/widgetjs/weather-source';
const provenance = [];
function preserve(source) {
  const bytes = readFileSync(upstream + '/' + source);
  const destination = target + '/original/' + source;
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(destination, bytes);
  provenance.push({ source, sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length });
  return bytes.toString('utf8');
}
mkdirSync(target + '/views', { recursive: true });
mkdirSync(target + '/assets', { recursive: true });
const views = [['weather/WeatherWidgetView.vue', 'Small'], ['weather-medium/WeatherMediumWidgetView.vue', 'Medium'], ['weather-large/WeatherLargeWidgetView.vue', 'Large']];
for (const [source, size] of views) {
  let text = preserve('src/widgets/' + source);
  text = text.replaceAll("'@icon-park/vue-next'", "'@/icons'")
    .replaceAll("'@widget-js/vue3'", "'@/context'")
    .replaceAll("'../../utils/StringUtils'", "'@/utils/StringUtils'")
    .replace(/:src="`\/weather\/image\/\$\{([^}]+)\}\.png`"/g, ':src="weatherImage($1)"')
    .replaceAll('/weather/image/', '../assets/')
    .replace('weather3dResponse, temperature }', 'weather3dResponse, temperature, forecastDayText }')
    .replace('StringUtils.getForecastDayText(index)', 'forecastDayText(item.fxDate)')
    .replace('<script lang="ts" setup>', '<script lang="ts" setup>\nimport { weatherImage } from \'@/images\'');
  writeFileSync(`${target}/views/${size}.vue`, text);
}
for (const file of readdirSync(upstream + '/public/image')) {
  if (!file.endsWith('.png')) throw Error('天气资源类型变化：' + file);
  preserve('public/image/' + file);
  copyFileSync(upstream + '/public/image/' + file, target + '/assets/' + file);
}
for (const source of ['src/hook/useQWeatherApi.ts','src/hook/useApiHost.ts','src/api/QWeatherApi.ts','src/api/GeoApi.ts','src/component/QWeatherWrapper.vue','src/utils/WeatherUtils.ts','src/utils/StringUtils.ts','src/assets/main.css']) {
  const text = preserve(source);
  if (source.includes('/utils/')) { mkdirSync(target + '/utils', { recursive: true }); writeFileSync(target + '/utils/' + source.split('/').at(-1), text); }
  if (source.endsWith('main.css')) writeFileSync(target + '/original-main.css', text);
}
const css = preserve('src/assets/qweather.css');
const viteRequire = createRequire(import.meta.resolve('vite'));
const postcss = viteRequire('postcss');
const backgrounds = new Set(['.city-weather-sun','.city-weather-cloudy','.city-weather-rain','.city-weather-snow','.city-weather-haze']);
const selected = [];
postcss.parse(css).walkRules(rule => { if (backgrounds.has(rule.selector)) selected.push(rule.toString()); });
if (selected.length !== backgrounds.size) throw Error('原天气渐变规则变化');
writeFileSync(target + '/original-backgrounds.css', selected.join('\n') + '\n');
writeFileSync(target + '/SOURCE_FILES.json', JSON.stringify({ repository:'https://github.com/rtugeek/weather', revision, files:provenance }, null, 2) + '\n');
console.log(JSON.stringify({ revision, views:3, images:72, files:provenance.length, destination:resolve(target) }));
