// 固定版本的源码导入与接口替换；不下载、不安装、不运行上游发布脚本。
import { readFileSync, writeFileSync, mkdirSync, copyFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
const source = 'artifacts/widgetjs-upstream/clock';
const target = 'components/widgetjs/clock-source';
const revision = '646bf72e584b946da424ced3d6253e0c71b8e6e1';
const actual = spawnSync('git', ['-C', source, 'rev-parse', 'HEAD'], { encoding: 'utf8' });
if (actual.status !== 0 || actual.stdout.trim() !== revision) throw new Error('时钟源码不是已审计的固定提交');
const files = [
  'widgets/clock/ClockWidgetView.vue', 'widgets/flip-clock/FlipClockWidgetView.vue',
  'widgets/glitch/GlitchWidget.vue', 'widgets/glitch/GlitchWidgetView.vue',
  'widgets/mickey-clock/MickeyClockWidget.vue', 'widgets/mickey-clock/MickeyClockWidgetView.vue',
  'widgets/text-clock/TextClockWidgetView.vue',
  'i18n/timezone.json', 'assets/font/FiraCode-Bold.woff2',
  ...['clock_bg.png', 'clock_hand_hour.png', 'clock_hand_minute.png', 'clock_hand_second.png'].map(name => 'widgets/clock/images/' + name),
];
for (const file of files) {
  const output = join(target, file);
  mkdirSync(dirname(output), { recursive: true });
  if (!file.endsWith('.vue')) { copyFileSync(join(source, 'src', file), output); continue; }
  let text = readFileSync(join(source, 'src', file), 'utf8')
    .replaceAll("'@widget-js/vue3'", "'@/souldesk'")
    .replaceAll('useWidgetStorage', 'useInstanceSetting')
    .replaceAll('useWidgetTheme', 'useComponentTheme')
    .replaceAll('useWidget', 'useComponent')
    .replaceAll('WidgetWrapper', 'ComponentFrame')
    .replaceAll('widget-wrapper', 'component-frame')
    .replaceAll("'show-timezone'", "'showTimezone'")
    .replaceAll('NotificationApi.info', 'showMessage')
    .replace("import { NotificationApi } from '@widget-js/core'", "import { showMessage } from '@/souldesk'")
    .replaceAll(/<style([^>]*)>/g, (_, attrs) => '<style' + attrs.replace(/\s+scoped/g, '') + ' scoped>')
    .replace(/@font-face\s*\{[\s\S]*?\}/, file.includes('flip-clock') ? '' : '$&')
    .replaceAll("'Saira ExtraCondensed'", "'FiraCode'")
    .replace("@import url('@/assets/font/firacode.css');", '');
  if (file.includes('clock/ClockWidgetView')) text = text.replace('useInterval(1000,', 'updateTime()\nuseInterval(1000,');
  if (file.includes('mickey-clock/MickeyClockWidget.vue')) {
    text = text.replace('setInterval(() => {', 'const update = () => {').replace('      }, 1000)', '      }\n      update()\n      setInterval(update, 1000)');
    text = text.replace('scale(0.28)', 'scale(var(--mickey-scale, 0.28))').replace('scale(0.23)', 'scale(var(--mickey-scale, 0.23))');
  }
  writeFileSync(output, '<!-- SoulDesk 本机自用移植；来源及修改见 SOURCE.txt。 -->\n' + text);
}
