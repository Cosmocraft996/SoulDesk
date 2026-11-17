// 从固定Git对象派生原模板/SCSS；不执行上游脚本，不引入Electron运行时。
import {mkdirSync,writeFileSync,readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
const repository='artifacts/widgetjs-upstream/utilities',revision='3d733215bac3e9c1e58125518f100c1601650828';
const source='src/widgets/key-stroke/KeyStrokeWidgetView.vue',target='components/widgetjs/key-stroke-source';
const result=spawnSync('git',['-C',repository,'show',`${revision}:${source}`],{maxBuffer:1024*1024});
if(result.status!==0)throw Error('无法读取固定键盘演示源码');
const sha256=createHash('sha256').update(result.stdout).digest('hex');
if(sha256!=='ead4faedf9b0a316e9d4377ac46bf574c8f0793b08b1224c148b4db917469927')throw Error('原Git对象散列不符');
mkdirSync(`${target}/original`,{recursive:true});writeFileSync(`${target}/original/KeyStrokeWidgetView.vue`,result.stdout);
const original=result.stdout.toString('utf8');
const script=`<script setup>
import {computed} from 'vue';
const props=defineProps({snapshot:{type:Object,required:true}});
const modifierKeys=computed(()=>props.snapshot.modifierKeys||[]),normalKey=computed(()=>props.snapshot.normalKey),capsLock=computed(()=>props.snapshot.capsLock),isCapsLockOn=computed(()=>props.snapshot.isCapsLockOn);
</script>`;
const overlay=original.replace(/<script[\s\S]*?<\/script>/,script)
  .replace('ref="keyboard"','')
  .replace('class="keyboard"','class="keyboard" :class="{\'is-visible\': snapshot.visible}"')
  .replace(/\s+@mouse(?:enter|leave)="[^"]*"/g,'')
  .replace('</style>',`.keyboard {opacity:0;transform:translateY(150px);transition:transform .3s cubic-bezier(.2,.8,.2,1),opacity .3s;pointer-events:none;}
.keyboard.is-visible {opacity:1;transform:translateY(0);}
</style>`);
writeFileSync(`${target}/Overlay.vue`,overlay);
writeFileSync(`${target}/SOURCE_FILES.json`,JSON.stringify({repository:'https://github.com/rtugeek/utilities',revision,files:[{source,sha256}],adaptation:'原Vue模板/SCSS，宿主快照替换钩子与窗口API；固定CSS代替motion；移除鼠标启用，保持穿透'},null,2)+'\n');
// 原仓库本次固定版本未提供LICENSE；自用范围按用户授权，保留可核验来源。
const manifest=JSON.parse(readFileSync('components/widgetjs/key-stroke/manifest.json','utf8'));
writeFileSync('components/widgetjs/key-stroke/SOURCE.txt',`来源：https://github.com/rtugeek/utilities\n固定提交：${revision}\n原文件：${source}\nSHA256（Git原始LF字节）：${sha256}\n原名称：快捷键敲击（BackgroundWidget）\n移植版本：${manifest.version}。显式启用宿主键盘演示，仅修饰键组合/CapsLock，无输入历史。原生与桌面效果待人工验收。\n原固定仓库未提供LICENSE；按用户确认仅本机自用，未授权发布上游代码。\n`);
console.log(JSON.stringify({revision,sha256,derived:'Overlay.vue'}));
