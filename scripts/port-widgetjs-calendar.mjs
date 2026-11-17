// 固定部署的纯历法模块直接保留；不执行WidgetJS入口。
import{readFileSync,writeFileSync,readdirSync}from'node:fs';import{createHash}from'node:crypto';
const root='components/widgetjs/calendar-source/';
const source=readFileSync(root+'original/CalendarWidgetView-tnDcscXu.css','utf8');
writeFileSync(root+'original-style.css',source.replaceAll(/\[data-v-[a-f0-9]+\]/g,''));
const files=readdirSync(root+'original').sort().map(name=>({path:'original/'+name,url:'https://itime.fun/web/assets/'+(name==='main-excerpts.txt'?'index-BJrdQu13.js':name),kind:name==='main-excerpts.txt'?'verbatim-excerpts':'deployed-asset',sha256:createHash('sha256').update(readFileSync(root+'original/'+name)).digest('hex')}));
writeFileSync(root+'SOURCE_FILES.json',JSON.stringify({entry:'https://itime.fun/web/',deployment:'index-BJrdQu13.js',entrySha256:'79ba25621b91084bb06fa4a3e78888eee28f03767949ac7049ab19b7dee5cf06',upstreamId:'itime.fun.calendar',publicCalendar:'https://itime.fun/api/v2/event/public/calendar',license:'部署资源未附许可声明，依用户明确的自用范围保留来源',files},null,2)+'\n');
console.log('日历原样式与10份来源记录已生成');
