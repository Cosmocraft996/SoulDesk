// 从已保存的固定部署生成本地静态资源，不执行上游宿主脚本。
import{readFileSync,writeFileSync,readdirSync}from'node:fs';import{createHash}from'node:crypto';
const root='components/widgetjs/countdown-source/';
const source=readFileSync(root+'original/RRuleUtils-DomKJTnx.js','utf8'),line='import{as as Pt}from"./index-BJrdQu13.js";';if(!source.startsWith(line))throw Error('重复算法入口发生变化');
writeFileSync(root+'recurrence.js',source.replace(line,'import Pt from"dayjs";'));
writeFileSync(root+'original-style.css',readFileSync(root+'original/CountdownWidgetView-6inb6o4W.css','utf8')+'\n'+readFileSync(root+'original/CountdownListWidgetView-C98whSZb.css','utf8').replaceAll(/\[data-v-[a-f0-9]+\]/g,''));
const files=readdirSync(root+'original').sort().map(name=>({path:'original/'+name,url:'https://itime.fun/web/assets/'+name,sha256:createHash('sha256').update(readFileSync(root+'original/'+name)).digest('hex')}));
writeFileSync(root+'SOURCE_FILES.json',JSON.stringify({deployment:'index-BJrdQu13.js',entry:'https://itime.fun/web/',upstreamIds:['itime.fun.countdown','itime.fun.countdown_list'],license:'部署资源未附许可，依用户明确自用范围保留来源',adaptations:{'recurrence.js':'仅将原入口dayjs导入改为现有本地dayjs，其余算法逐字保持','original-style.css':'合并原单卡与列表样式，去除Vue编译作用域属性'},files},null,2)+'\n');
console.log('倒计时原重复算法、样式与来源已生成');
