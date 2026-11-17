import dayjs from'dayjs';import{L as Lunar,S as Solar,b as LunarMonth}from'./original/index-foMHgrIw.js';import{a as RRule}from'./recurrence.js';
export const defaults={background:'#ffffff',foreground:'#ffffff',primaryColor:'rgb(0,149,255)',radius:8};
const clone=value=>structuredClone(value),object=value=>value&&typeof value==='object'&&!Array.isArray(value);
const color=value=>typeof value==='string'&&value.length<=80&&(/^(?:#[a-f\d]{3,4}|#[a-f\d]{6}|#[a-f\d]{8}|transparent)$/i.test(value)||/^(?:rgb|hsl)a?\([\d.% ,/+-]+\)$/i.test(value));
function text(value,label,max){if(typeof value!=='string'||value.length>max)throw Error(label+'超出'+max+'字或格式无效');return value;}
function iso(value){if(typeof value!=='string'||!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value)||!Number.isFinite(Date.parse(value))||new Date(value).toISOString()!==value)throw Error('日期格式无效');return value;}
function rule(value){if(!['','RRULE:FREQ=DAILY','RRULE:FREQ=WEEKLY','RRULE:FREQ=MONTHLY','RRULE:FREQ=YEARLY'].includes(value))throw Error('重复规则无效');return value;}
function validateEvent(value){
  if(!object(value)||!text(value.id,'标识',128)||!text(value.name,'名称',15).trim())throw Error('名称或标识无效');text(value.note,'备注',4096);iso(value.sourceDateTime);iso(value.dateTime);if(![0,1].includes(value.dateType))throw Error('日期类型无效');rule(value.recurrence);if(value.dateType===1&&value.recurrence)throw Error('原农历倒计时不设置重复');
  for(const key of['createdAt','updatedAt','sortOrder'])if(!Number.isSafeInteger(value[key])||value[key]<0)throw Error('排序或保存时间无效');return value;
}
export function validateSettings(raw){const value={...defaults,...raw};for(const key of['background','foreground','primaryColor'])if(!color(value[key]))throw Error('颜色格式无效');if(!Number.isInteger(value.radius)||value.radius<0||value.radius>50)throw Error('圆角须为0～50的整数');return value;}
export function validateDocument(raw,limit=4*1024*1024){
  if(!object(raw)||raw.version!==1||!Array.isArray(raw.events)||raw.events.length>512||!['asc','desc'].includes(raw.sort)||typeof raw.selectedId!=='string')throw Error('倒计时存储格式无效，请保留原数据');
  const ids=new Set();for(const item of raw.events){validateEvent(item);if(ids.has(item.id))throw Error('倒计时标识重复');ids.add(item.id);}if(raw.selectedId&&!ids.has(raw.selectedId))throw Error('所选倒计时不存在');if(new TextEncoder().encode(JSON.stringify(raw)).length>limit)throw Error('倒计时超过实例存储上限');return clone(raw);
}
export function restoreDocument(raw,now=Date.now(),id=()=>crypto.randomUUID()){
  if(!object(raw))throw Error('倒计时存储格式无效');if(Object.keys(raw).length)return validateDocument(raw);
  const next=new Date(now);next.setFullYear(next.getFullYear()+1,0,1);next.setHours(0,0,0,0);const value={id:id(),name:'新年',note:'',sourceDateTime:next.toISOString(),dateTime:next.toISOString(),dateType:0,recurrence:'',sortOrder:0,createdAt:now,updatedAt:now};return validateDocument({version:1,events:[value],selectedId:value.id,sort:'asc'});
}
export function viewEvent(raw,now=Date.now()){
  validateEvent(raw);if(!Number.isFinite(now))throw Error('当前时间无效');let target=dayjs(raw.sourceDateTime);const current=dayjs(now),today=current.startOf('day');
  if(raw.recurrence&&target.isBefore(today)){
    // 日期型事件以本地年月日喂给原算法；直接传UTC瞬间会把东八区每月1日变成UTC每月31日。
    const floating=value=>{const date=new Date(0);date.setUTCFullYear(value.year(),value.month(),value.date());date.setUTCHours(0,0,0,0);return date;};
    const opts=RRule.parseString(raw.recurrence),next=new RRule({...opts,dtstart:floating(target)}).after(floating(today),true);if(!next)throw Error('重复规则没有下次日期');const date=new Date(0);date.setFullYear(next.getUTCFullYear(),next.getUTCMonth(),next.getUTCDate());date.setHours(0,0,0,0);target=dayjs(date);
  }
  const same=target.format('YYYY-MM-DD')===current.format('YYYY-MM-DD'),cardDays=Math.ceil(target.diff(current,'day',true)),days=same?0:cardDays,dateTime=target.toISOString(),solar=Solar.fromDate(target.toDate()),lunar=Lunar.fromDate(target.toDate());
  return{...clone(raw),dateTime,days,cardDays,today:same,past:days<0,dateText:raw.dateType===1?lunar.toString():target.format('YYYY年MM月DD日'),cardDateText:raw.dateType===1?lunar.toString():target.format('YYYY/MM/DD'),solarYear:solar.getYear(),solarMonth:solar.getMonth(),solarDay:solar.getDay()};
}
export function eventDraft(item){const date=new Date(item.sourceDateTime),lunar=Lunar.fromDate(date);return{id:item.id,name:item.name,note:item.note,dateType:item.dateType,date:dayjs(date).format('YYYY-MM-DD'),year:lunar.getYear(),month:lunar.getMonth(),dayOfMonth:lunar.getDay(),recurrence:item.recurrence};}
export function eventFromDraft(draft,previous,now=Date.now(),id=()=>crypto.randomUUID()){
  let source;if(draft.dateType===1){const y=Number(draft.year),m=Number(draft.month),d=Number(draft.dayOfMonth);try{if(!Number.isInteger(y)||y<1949||y>2099||!Number.isInteger(m)||m===0||Math.abs(m)>12||!Number.isInteger(d)||d<1)throw Error();const month=LunarMonth.fromYm(y,m);if(!month||d>month.getDayCount())throw Error();const solar=Lunar.fromYmd(y,m,d).getSolar();source=new Date(solar.getYear(),solar.getMonth()-1,solar.getDay());}catch{throw Error('农历日期不存在或超出1949～2099年范围');}}
  else{if(draft.dateType!==0||typeof draft.date!=='string'||!/^\d{4}-\d\d-\d\d$/.test(draft.date))throw Error('日期格式无效');const[y,m,d]=draft.date.split('-').map(Number);source=new Date(0);source.setHours(0,0,0,0);source.setFullYear(y,m-1,d);if(y<1||source.getFullYear()!==y||source.getMonth()!==m-1||source.getDate()!==d)throw Error('日期不存在');}
  const target=source.toISOString(),value={...(previous||{}),id:previous?.id||id(),name:String(draft.name??'').trim(),note:draft.note??'',dateType:draft.dateType,recurrence:draft.dateType===1?'':draft.recurrence||'',sourceDateTime:target,dateTime:target,sortOrder:previous?.sortOrder??0,createdAt:previous?.createdAt??now,updatedAt:now};return clone(validateEvent(value));
}
export function ordered(document,now=Date.now()){return document.events.map(event=>viewEvent(event,now)).sort((a,b)=>a.sortOrder-b.sortOrder||(document.sort==='asc'?Date.parse(a.dateTime)-Date.parse(b.dateTime):Date.parse(b.dateTime)-Date.parse(a.dateTime)));}
export function reorder(document,from,to){const next=validateDocument(document);if(from===to)return next;const values=next.events.map(event=>({...event})),a=values.findIndex(item=>item.id===from);if(a<0||!values.some(item=>item.id===to))throw Error('拖动项目不存在');const item=values.splice(a,1)[0],b=values.findIndex(item=>item.id===to);values.splice(b,0,item);next.events=values.map((event,index)=>({...event,sortOrder:index}));return validateDocument(next);}
export function createStore(initial,persist,{preview=false,changed=()=>{}}={}){
  let value=validateDocument(initial),tail=Promise.resolve(),latest=tail,disposed=false;
  function update(change){const job=tail.then(async()=>{if(disposed)throw Error('组件已关闭');if(preview)throw Error('预览不保存倒计时');const next=clone(value);const result=change(next);if(result&&typeof result.then==='function')throw Error('不接受异步数据修改');const valid=validateDocument(next);await persist(valid);if(!disposed){value=valid;changed(clone(value));}return clone(valid);});latest=job;tail=job.catch(()=>{});return job;}
  return{get:()=>clone(value),update,flush:()=>latest,dispose(){disposed=true;}};
}
