// 固定部署资源只作为源码和静态资源，不执行其宿主入口。
import{readFileSync,writeFileSync,readdirSync}from'node:fs';import{createHash}from'node:crypto';
const root='components/widgetjs/birthday-list-source/';
writeFileSync(root+'original-style.css',readFileSync(root+'original/BirthdayListWidgetView-DZdE-yIq.css','utf8').replaceAll(/\[data-v-[a-f0-9]+\]/g,''));
const files=readdirSync(root+'original').sort().map(name=>({path:'original/'+name,url:'https://itime.fun/web/assets/'+name,sha256:createHash('sha256').update(readFileSync(root+'original/'+name)).digest('hex')}));
writeFileSync(root+'SOURCE_FILES.json',JSON.stringify({deployment:'index-BJrdQu13.js',entry:'https://itime.fun/web/',upstreamId:'itime.fun.birthday_list',license:'公开部署未附许可声明，依用户明确自用范围保留来源',files},null,2)+'\n');
console.log('生日原样式及来源记录已生成');
