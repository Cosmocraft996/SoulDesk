// 从固定部署静态提取原FullCalendar；不执行原组件、云账户或宿主入口。
import{readFileSync,writeFileSync,readdirSync}from'node:fs';import{createHash}from'node:crypto';import ts from'typescript';
const root='components/widgetjs/calendar-large-source/',file='CalendarLargeWidgetView-BWZiw04f.js',source=readFileSync(root+'original/'+file,'utf8');
if(createHash('sha256').update(Buffer.from(source)).digest('hex')!=='727c3e3a2e8d19d2967ca27bb40d4cc25bc4e20513785975b93b19307669299d')throw Error('原大日历模块散列变化，停止提取');
const ast=ts.createSourceFile(file,source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS),edits=[],css=[];let end=0;
for(const statement of ast.statements){
 if(ts.isImportDeclaration(statement))edits.push({start:statement.getStart(ast),end:statement.end,text:statement.moduleSpecifier.text==='./RRuleUtils-DomKJTnx.js'?"import{b as Il,r as xl,a as Ni}from'../countdown-source/recurrence.js';":''});
 if(ts.isFunctionDeclaration(statement)&&statement.name.text==='Hh'){end=statement.getStart(ast);break;}
 if(ts.isFunctionDeclaration(statement)&&['es','ts','$d','Xd'].includes(statement.name.text))edits.push({start:statement.getStart(ast),end:statement.end,text:['es','ts'].includes(statement.name.text)?'function '+statement.name.text+'(){}':''});
 if(ts.isVariableStatement(statement)&&statement.declarationList.declarations.some(d=>d.name.getText(ast)==='Qd'))edits.push({start:statement.getStart(ast),end:statement.end,text:''});
}
if(!end)throw Error('未定位原引擎边界');
function visit(node){if(node.getStart(ast)>=end)return;if(ts.isStringLiteral(node)&&node.text.includes('.fc-')&&node.text.length>500){css.push(node.text);edits.push({start:node.getStart(ast),end:node.end,text:'""'});}ts.forEachChild(node,visit);}visit(ast);
if(css.length!==2)throw Error('FullCalendar样式块数量变化');
let output=source.slice(0,end);for(const change of edits.filter(e=>e.end<=end).sort((a,b)=>b.start-a.start))output=output.slice(0,change.start)+change.text+output.slice(change.end);
output+='\nexport{Gd as Calendar,Tf as dayGridPlugin,Nf as rrulePlugin,Yh as interactionPlugin};\n';
writeFileSync(root+'fullcalendar.js','// 固定部署的FullCalendar引擎；原宿主和Vue包装已移除，CSS独立交付。\n'+output);
writeFileSync(root+'fullcalendar.css',css.join('\n')+'\n');writeFileSync(root+'original-style.css',readFileSync(root+'original/CalendarLargeWidgetView-CeVf48fi.css','utf8'));
const files=readdirSync(root+'original').sort().map(name=>{const bytes=readFileSync(root+'original/'+name);return{path:'original/'+name,url:'https://itime.fun/web/assets/'+name,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};});
writeFileSync(root+'SOURCE_FILES.json',JSON.stringify({entry:'https://itime.fun/web/',deployment:'index-BJrdQu13.js',entrySha256:'79ba25621b91084bb06fa4a3e78888eee28f03767949ac7049ab19b7dee5cf06',upstreamId:'itime.fun.calendar_large',license:'公开部署未附许可声明，按用户明确自用范围保留来源',adaptation:'保留原FullCalendar/Preact及daygrid/rrule/interaction算法；移除WidgetJS/Vue包装，原两段CSS独立交付，停用运行时样式注入',files},null,2)+'\n');
console.log('已提取原FullCalendar、两段CSS及'+files.length+'份来源记录');
