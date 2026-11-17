import test from'node:test';import assert from'node:assert/strict';import{existsSync,readFileSync}from'node:fs';import{createHash}from'node:crypto';import ts from'typescript';
const root='components/widgetjs/calendar-large-source/';
test('大号日历保留固定FullCalendar引擎与插件，移除WidgetJS宿主和动态样式注入',async()=>{
 assert.ok(existsSync(root+'fullcalendar.js'),'原FullCalendar引擎尚未提取');
 const original=readFileSync(root+'original/CalendarLargeWidgetView-BWZiw04f.js');assert.equal(createHash('sha256').update(original).digest('hex'),'727c3e3a2e8d19d2967ca27bb40d4cc25bc4e20513785975b93b19307669299d');
 const text=readFileSync(root+'fullcalendar.js','utf8'),ast=ts.createSourceFile('fullcalendar.js',text,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS),imports=ast.statements.filter(ts.isImportDeclaration).map(st=>st.moduleSpecifier.text);
 assert.deepEqual(imports,['../countdown-source/recurrence.js']);assert.ok(!text.includes('index-BJrdQu13'));assert.ok(!text.includes('CalendarLargeWidgetView'));assert.match(text,/class Gd extends Ac/);
 for(const name of ['es','ts']){const declaration=ast.statements.find(st=>ts.isFunctionDeclaration(st)&&st.name.text===name);assert.equal(declaration.body.statements.length,0,'静态CSS已提取，禁止运行时注入样式');}
 const mod=await import('../components/widgetjs/calendar-large-source/fullcalendar.js');assert.equal(typeof mod.Calendar,'function');assert.equal(mod.dayGridPlugin.name,'@fullcalendar/daygrid');assert.equal(mod.rrulePlugin.name,'@fullcalendar/rrule');assert.equal(mod.interactionPlugin.name,'@fullcalendar/interaction');
 const css=readFileSync(root+'fullcalendar.css','utf8');assert.ok(css.length>17000);assert.match(css,/\.fc-daygrid-day-events/);assert.match(css,/\.fc-toolbar/);
});
