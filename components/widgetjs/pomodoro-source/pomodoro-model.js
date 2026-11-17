const copy=v=>JSON.parse(JSON.stringify(v)),uuid=/^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}$/i;
export const templates=[{id:'reading',name:'阅读',icon:'📖'},{id:'coding',name:'敲代码',icon:'👨‍💻'},{id:'working',name:'打工',icon:'🔨'},{id:'relaxing',name:'摸鱼',icon:'🐟'}];
export function emptyWindowSnapshot(){return{revision:0,config:{enabled:false},surface:null,nativeCleanupPending:false,nativeError:null};}
function validateSurface(s){if(s===null)return s;if(!s||!uuid.test(s.presentationId)||!['bottom','top','left','right'].includes(s.edge)||['expanded','hidden','suspended','ready','nativeSettled'].some(k=>typeof s[k]!=='boolean')||!(s.fault===null||typeof s.fault==='string'))throw Error('番茄专用窗状态无效');return copy(s);}
export function validateWindowSnapshot(s){if(!s||!Number.isSafeInteger(s.revision)||s.revision<0||typeof s.config?.enabled!=='boolean'||typeof s.nativeCleanupPending!=='boolean'||!(s.nativeError===null||typeof s.nativeError==='string'))throw Error('番茄专用窗设置版本无效');return{...copy(s),surface:validateSurface(s.surface)};}
/** 专用窗许可为每实例私有配置；未知保存结果只重发同一个请求。 */
export function createWindowControl(initial,request,{preview=false,changed=()=>{}}={}){
 let value=validateWindowSnapshot(initial),pending=null,alive=true,last=Promise.resolve();
 const publish=next=>{next=validateWindowSnapshot(next);if(next.revision<value.revision)throw Error('收到旧版本专用窗设置');if(alive){value=next;changed(copy(next));}return copy(next);};
 const queue=run=>{const p=last.then(run,run);last=p.catch(()=>{});return p;};
 async function submit(){try{const result=publish(await request('pomodoro.window.configure',copy(pending)));pending=null;return result;}catch(e){if(/^(INVALID_ARGUMENT|REVISION_CONFLICT|CAPABILITY_DENIED|SESSION_EXPIRED|PRESENTATION_EXPIRED|DATA_UNAVAILABLE):/.test(String(e?.message||e))){pending=null;if(String(e?.message||e).startsWith('REVISION_CONFLICT:'))try{publish(await request('pomodoro.window.status'));}catch{}}throw e;}}
 return{get value(){return copy(value);},hasPending:()=>!!pending,flush:()=>last,
  configure(enabled,expectedRevision=value.revision){return queue(()=>{if(preview)throw Error('预览不启用专用窗');if(pending)throw Error('上次保存结果未确定，请重试原操作');if(typeof enabled!=='boolean'||!Number.isSafeInteger(expectedRevision)||expectedRevision<0)throw Error('专用窗开关或版本无效');pending={requestId:crypto.randomUUID(),expectedRevision,config:{enabled}};return submit();});},
  retry(){return queue(()=>{if(preview||!pending)throw Error('没有待重试的专用窗设置');return submit();});},
  refresh(){return queue(async()=>preview||pending?copy(value):publish(await request('pomodoro.window.status')));},
  action(action){return queue(async()=>{if(preview||pending)throw Error('请先确定专用窗设置');if(!['open','hide','reset'].includes(action))throw Error('专用窗动作无效');return publish(await request('pomodoro.window.'+action));});},
  surface(next){if(preview||!alive)return;next=validateSurface(next);if(!next||value.surface?.presentationId!==next.presentationId)return;value={...value,surface:next};changed(copy(value));},
  dispose(){alive=false;}
 };
}
export function emptySnapshot(){return{version:1,revision:0,initialized:false,sampledAt:0,config:{pomoTime:30,shortBreakTime:5,isAutoNext:true,isSoundEnabled:true},scenes:[],history:[],currentSceneId:null,phase:{status:'stop',cycleId:null,sceneId:null,startedAt:0,phaseStartedAt:null,focusMillis:0,restMillis:0},remainingSeconds:1800,progress:100,notice:null,message:null};}
export function validateConfig(c){if(!c||!Number.isInteger(c.pomoTime)||c.pomoTime<5||c.pomoTime>60||!Number.isInteger(c.shortBreakTime)||c.shortBreakTime<1||c.shortBreakTime>30||typeof c.isAutoNext!=='boolean'||typeof c.isSoundEnabled!=='boolean')throw Error('专注须为5～60分钟，休息须为1～30分钟');return copy(c);}
export function validateSnapshot(s){
 if(!s||s.version!==1||!Number.isSafeInteger(s.revision)||s.revision<0||typeof s.initialized!=='boolean'||!Number.isSafeInteger(s.sampledAt)||!Array.isArray(s.scenes)||s.scenes.length>128||!Array.isArray(s.history)||s.history.length>8192||!s.phase||!['stop','running','pause','rest-pause','waiting','resting'].includes(s.phase.status)||!Number.isSafeInteger(s.remainingSeconds)||s.remainingSeconds<0)throw Error('番茄钟数据版本或状态无效，未覆盖');validateConfig(s.config);
 const ids=new Set();for(const v of s.scenes){if(!uuid.test(v.id)||ids.has(v.id)||typeof v.name!=='string'||!v.name.trim()||[...v.name].length>120||typeof v.icon!=='string'||![...v.icon].length||[...v.icon].length>32||!Number.isInteger(v.sortOrder))throw Error('场景数据无效');ids.add(v.id);}
 if(s.currentSceneId&&!ids.has(s.currentSceneId)||s.phase.sceneId&&!ids.has(s.phase.sceneId))throw Error('当前场景无效');const records=new Set();for(const h of s.history){if(!uuid.test(h.id)||records.has(h.id)||!ids.has(h.sceneId)||!Number.isSafeInteger(h.duration)||h.duration<60||h.duration>3600||!Number.isSafeInteger(h.startTime)||!Number.isSafeInteger(h.finishTime)||h.finishTime<h.startTime)throw Error('专注记录无效');records.add(h.id);}
 return copy(s);
}
export function clockText(seconds){if(!Number.isSafeInteger(seconds)||seconds<0)throw Error('计时无效');return String(Math.floor(seconds/60)).padStart(2,'0')+':'+String(seconds%60).padStart(2,'0');}
const pad=n=>String(n).padStart(2,'0');
export function dayKey(time){const d=new Date(time);return d.getFullYear()+'-'+pad(d.getMonth()+1)+'-'+pad(d.getDate());}
export function recordTime(time){const d=new Date(time);return dayKey(time)+' '+pad(d.getHours())+':'+pad(d.getMinutes());}
export function sceneStats(history,id){const items=history.filter(h=>h.sceneId===id);return{totalHours:(items.reduce((n,h)=>n+h.duration,0)/3600).toFixed(1),checkInDays:new Set(items.map(h=>dayKey(h.startTime))).size,count:items.length};}
export function monthStats(history,id,month,now=Date.now()){
 const start=new Date(month.getFullYear(),month.getMonth(),1),today=new Date(now),same=start.getFullYear()===today.getFullYear()&&start.getMonth()===today.getMonth(),count=same?today.getDate():new Date(start.getFullYear(),start.getMonth()+1,0).getDate();
 const seconds=new Map();for(const h of history)if(h.sceneId===id){const key=dayKey(h.startTime);seconds.set(key,(seconds.get(key)||0)+h.duration);}
 const days=Array.from({length:count},(_,i)=>{const date=dayKey(new Date(start.getFullYear(),start.getMonth(),i+1));return{date,day:i+1,minutes:Math.ceil((seconds.get(date)||0)/60)};});return{days,totalMinutes:days.reduce((n,d)=>n+d.minutes,0),checkInDays:days.filter(d=>d.minutes>0).length,maxMinutes:Math.max(0,...days.map(d=>d.minutes))};
}
export function historyPage(history,id,page=1){const items=history.filter(h=>h.sceneId===id).sort((a,b)=>b.startTime-a.startTime||b.id.localeCompare(a.id)),pages=Math.max(1,Math.ceil(items.length/20));page=Math.max(1,Math.min(pages,page));return{items:items.slice((page-1)*20,page*20),page,pages,total:items.length};}
export function createStore(initial,request,{preview=false,changed=()=>{}}={}){
 let value=validateSnapshot(initial),alive=true,tail=Promise.resolve(),pending=null,last=Promise.resolve();
 const publish=next=>{const valid=validateSnapshot(next);if(valid.revision<value.revision)throw Error('收到旧版本番茄钟数据');if(alive){value=valid;changed(copy(value));}return copy(value);};
 const queue=work=>{const result=tail.then(()=>{if(!alive)throw Error('视图已关闭');return work();});last=result;tail=result.catch(()=>{});return result;};
 async function submit(){try{const next=publish(await request('pomodoro.change',copy(pending)));pending=null;return next;}catch(e){if(/^(INVALID_ARGUMENT|REVISION_CONFLICT|CAPABILITY_DENIED|SESSION_EXPIRED|DATA_UNAVAILABLE):/.test(String(e?.message||e))){pending=null;if(String(e?.message||e).startsWith('REVISION_CONFLICT:'))try{publish(await request('pomodoro.get'));}catch{}}throw e;}}
 return{flush(){return last;},get value(){return copy(value);},hasPending:()=>!!pending,change(operation,payload={}){return queue(()=>{if(preview)throw Error('预览不保存番茄钟');if(pending)throw Error('上次操作结果未确定，请先重试');pending={requestId:crypto.randomUUID(),expectedRevision:value.revision,operation,...copy(payload)};return submit();});},retry(){return queue(()=>{if(preview)throw Error('预览不保存番茄钟');if(!pending)throw Error('没有待重试操作');return submit();});},refresh(){return queue(async()=>preview||pending?copy(value):publish(await request('pomodoro.get')));},dispose(){alive=false;}};
}
