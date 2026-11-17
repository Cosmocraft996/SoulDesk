// 只适配固定待办接口；提醒调度及完成后的重复生成始终由核心负责。
const copy=value=>JSON.parse(JSON.stringify(value)),uuid=/^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}$/i;
const rules=['','RRULE:FREQ=DAILY','RRULE:FREQ=WEEKLY','RRULE:FREQ=MONTHLY','RRULE:FREQ=YEARLY'];
export function repeatLabel(value){return({'RRULE:FREQ=DAILY':'每天重复','RRULE:FREQ=WEEKLY':'每周重复','RRULE:FREQ=MONTHLY':'每月重复','RRULE:FREQ=YEARLY':'每年重复'})[value]||'';}
const pad=n=>String(n).padStart(2,'0');
export function localDate(value){const d=new Date(value);if(!Number.isFinite(d.getTime()))throw Error('日期无效');return String(d.getFullYear()).padStart(4,'0')+'-'+pad(d.getMonth()+1)+'-'+pad(d.getDate());}
export function localMinute(value){const d=new Date(value);return localDate(d)+'T'+pad(d.getHours())+':'+pad(d.getMinutes());}
function iso(value){if(typeof value!=='string'||!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,3})?Z$/.test(value))throw Error('待办日期无效');const d=new Date(value);if(!Number.isFinite(d.getTime())||d.toISOString().slice(0,19)!==value.slice(0,19))throw Error('待办日期无效');return d.toISOString();}
function parseDay(value,clock){const match=/^(\d{4})-(\d\d)-(\d\d)$/.exec(value);if(!match)throw Error('日期无效');const [year,month,day]=match.slice(1).map(Number),d=new Date(clock);d.setFullYear(year,month-1,day);if(year<1||d.getFullYear()!==year||d.getMonth()!==month-1||d.getDate()!==day)throw Error('日期无效');return d;}
function parseMinute(value){const m=/^(\d{4}-\d\d-\d\d)T(\d\d):(\d\d)$/.exec(value);if(!m||Number(m[2])>23||Number(m[3])>59)throw Error('提醒时间无效');const d=parseDay(m[1],0);d.setHours(Number(m[2]),Number(m[3]),0,0);if(localMinute(d)!==value)throw Error('提醒时间在系统时区中不存在');return d;}
export function defaultReminder(now){const d=new Date(now+3600000);d.setMinutes(0,0,0);return localMinute(d);}
export function newDraft(now=Date.now()){return{id:crypto.randomUUID(),title:'',dueDate:'',recurrence:'',isReminderOn:false,reminder:defaultReminder(now)};}
export function toDraft(t){return{id:t.id,title:t.title,dueDate:t.dueDateTime?localDate(t.dueDateTime):'',recurrence:t.recurrence||'',isReminderOn:t.isReminderOn,reminder:t.reminderDateTime?localMinute(t.reminderDateTime):''};}
export function fromDraft(d,previous,now=Date.now()){
 if(!uuid.test(d.id)||typeof d.title!=='string'||!d.title.trim()||[...d.title].length>4096)throw Error('请填写1～4096字的待办内容');
 if(!rules.includes(d.recurrence||''))throw Error('重复规则无效');if(d.recurrence&&!d.dueDate)throw Error('重复待办需要到期日期');
 const t={id:d.id,title:d.title,recurrence:d.recurrence||'',isReminderOn:d.isReminderOn===true,importance:previous?.importance||'normal',order:previous?.order||0};
 for(const key of ['createdDateTime','lastModifiedDateTime','startDateTime'])if(previous?.[key])t[key]=previous[key];
 if(d.dueDate)t.dueDateTime=parseDay(d.dueDate,previous?.dueDateTime?new Date(previous.dueDateTime).getTime():now).toISOString();
 if(d.reminder){t.reminderDateTime=previous?.reminderDateTime&&localMinute(previous.reminderDateTime)===d.reminder?previous.reminderDateTime:parseMinute(d.reminder).toISOString();}
 if(t.isReminderOn&&(!t.reminderDateTime||Date.parse(t.reminderDateTime)<=now))throw Error('提醒时间必须晚于当前时间');
 return t;
}
export function validateSettings(s){if(!s||typeof s!=='object'||!['title','background','foreground','divider'].every(k=>typeof s[k]==='string'&&s[k].length<=(k==='title'?120:80))||!['background','foreground','divider'].every(k=>s[k].trim())||!Number.isInteger(s.radius)||s.radius<0||s.radius>50)throw Error('待办标题或外观设置无效');if(s.firstDayOfWeek!==undefined&&![0,1].includes(s.firstDayOfWeek))throw Error('一周开始日无效');return{title:s.title,background:s.background,foreground:s.foreground,divider:s.divider,radius:s.radius,...(s.firstDayOfWeek===undefined?{}:{firstDayOfWeek:s.firstDayOfWeek})};}
export function validateSnapshot(value){
 if(!value||value.version!==1||!Number.isSafeInteger(value.revision)||value.revision<0||!Number.isSafeInteger(value.sampledAt)||!Array.isArray(value.todos)||value.todos.length>512||!Array.isArray(value.reminders)||value.reminders.length>512)throw Error('待办数据版本或内容无效，未覆盖原数据');
 const ids=new Set();for(const t of value.todos){if(!uuid.test(t.id)||ids.has(t.id)||typeof t.title!=='string'||!t.title.trim()||[...t.title].length>4096||typeof t.isReminderOn!=='boolean'||!rules.includes(t.recurrence)||!Number.isInteger(t.order)||t.order<0||!['normal','high','low'].includes(t.importance))throw Error('待办事项数据无效');ids.add(t.id);for(const key of ['dueDateTime','reminderDateTime','completedDateTime','startDateTime','createdDateTime','lastModifiedDateTime'])if(t[key]!=null)iso(t[key]);if(t.recurrence&&!t.dueDateTime||t.isReminderOn&&!t.reminderDateTime)throw Error('待办日期缺失');}
 for(const r of value.reminders)if(!ids.has(r.todoId)||!Number.isSafeInteger(r.at)||!['waiting','sending','accepted','failed','unknown','expired','dismissed'].includes(r.delivery)||r.error!=null&&typeof r.error!=='string')throw Error('待办提醒状态无效');
 if(new TextEncoder().encode(JSON.stringify(value)).length>4*1024*1024)throw Error('待办数据超过4 MiB');return copy(value);
}
export function moveBefore(snapshot,from,to){const ids=snapshot.todos.filter(t=>!t.completedDateTime).map(t=>t.id);if(!ids.includes(from)||to&&!ids.includes(to))throw Error('排序事项无效');if(from===to)return ids;ids.splice(ids.indexOf(from),1);ids.splice(to?ids.indexOf(to):ids.length,0,from);return ids;}
export function createStore(initial,request,{preview=false,changed=()=>{}}={}){
 let value=validateSnapshot(initial),alive=true,tail=Promise.resolve(),pending=null,last=Promise.resolve();
 const publish=next=>{const validated=validateSnapshot(next);if(alive){if(validated.revision<value.revision)throw Error('收到旧版本待办数据');value=validated;changed(copy(value));}return copy(value);};
 const queue=work=>{const task=tail.then(()=>{if(!alive)throw Error('视图已关闭');return work();});last=task;tail=task.catch(()=>{});return task;};
 async function submit(){try{const result=await request('todo.change',copy(pending));const next=publish(result);pending=null;return next;}catch(reason){if(/^(INVALID_ARGUMENT|REVISION_CONFLICT|CAPABILITY_DENIED|SESSION_EXPIRED|DATA_UNAVAILABLE):/.test(String(reason?.message||reason))){pending=null;if(String(reason?.message||reason).startsWith('REVISION_CONFLICT:')){try{publish(await request('todo.get'));}catch{}}}throw reason;}}
 return{flush(){return last;},get value(){return copy(value);},hasPending:()=>pending!==null,change(operation,payload={}){return queue(async()=>{if(preview)throw Error('预览不保存待办');if(pending)throw Error('上次操作结果未确定，请先重试');pending={requestId:crypto.randomUUID(),expectedRevision:value.revision,operation,...copy(payload)};return submit();});},retry(){return queue(()=>{if(preview)throw Error('预览不保存待办');if(!pending)throw Error('没有需要重试的操作');return submit();});},refresh(){return queue(async()=>preview||pending?copy(value):publish(await request('todo.get')));},dispose(){alive=false;}};
}
