import dayjs from'dayjs';import{L as Lunar,b as LunarMonth,a as LunarYear}from'./original/index-foMHgrIw.js';
export{LunarYear};
const copy=value=>structuredClone(value),object=value=>value&&typeof value==='object'&&!Array.isArray(value);
export const defaults={background:'rgba(0,0,0,0.5)'};
function text(value,label,max){if(typeof value!=='string'||value.length>max)throw Error(label+'格式无效或超过'+max+'字');return value;}
function number(value,min,max,label){if(!Number.isInteger(value)||value<min||value>max)throw Error(label+'无效');return value;}
function date(year,month,day){const result=new Date(0);result.setFullYear(year,month-1,day);result.setHours(0,0,0,0);return result;}
function sourceDate(value){
  number(value.year,1,9999,'年份');number(value.dayOfMonth,1,31,'日期');
  if(value.dateType===0){number(value.month,1,12,'月份');const result=date(value.year,value.month,value.dayOfMonth);if(result.getFullYear()!==value.year||result.getMonth()!==value.month-1||result.getDate()!==value.dayOfMonth)throw Error('出生日期不存在');return result;}
  if(value.dateType!==1||value.year<1949||value.year>2099||!Number.isInteger(value.month)||value.month===0||Math.abs(value.month)>12)throw Error('农历日期无效');
  const month=LunarMonth.fromYm(value.year,value.month);if(!month||value.dayOfMonth>month.getDayCount())throw Error('农历出生日期不存在');const solar=Lunar.fromYmd(value.year,value.month,value.dayOfMonth).getSolar();return date(solar.getYear(),solar.getMonth(),solar.getDay());
}
function validateBirthday(value){if(!object(value)||!text(value.id,'标识',128)||!text(value.name,'姓名',120).trim())throw Error('姓名或标识无效');text(value.introduction,'介绍',4096);sourceDate(value);for(const key of['sortOrder','createdAt','updatedAt'])number(value[key],0,Number.MAX_SAFE_INTEGER,'保存时间或排序');return value;}
export function validateSettings(raw){const value={...defaults,...raw},color=value.background;if(typeof color!=='string'||color.length>80||!(/^(?:#[a-f\d]{3,4}|#[a-f\d]{6}|#[a-f\d]{8}|transparent)$/i.test(color)||/^(?:rgb|hsl)a?\([\d.% ,/+-]+\)$/i.test(color)))throw Error('背景颜色格式无效');return value;}
export function validateDocument(raw,limit=4*1024*1024){if(!object(raw)||raw.version!==1||!Array.isArray(raw.birthdays)||raw.birthdays.length>512)throw Error('生日存储格式无效，请保留原数据');const ids=new Set();for(const value of raw.birthdays){validateBirthday(value);if(ids.has(value.id))throw Error('生日标识重复');ids.add(value.id);}if(new TextEncoder().encode(JSON.stringify(raw)).length>limit)throw Error('生日数据超过实例存储上限');return copy(raw);}
export function restoreDocument(raw){if(!object(raw))throw Error('生日存储格式无效');return Object.keys(raw).length?validateDocument(raw):{version:1,birthdays:[]};}
function solarNext(value,now){const current=new Date(now);let target=date(current.getFullYear(),value.month,value.dayOfMonth);if(dayjs(target).isBefore(dayjs(current).startOf('day')))target=date(current.getFullYear()+1,value.month,value.dayOfMonth);return target;}
function lunarDate(year,month,day){
  // 用户确认：目标年无出生同名闰月时采用普通同月，保留出生月份本身。
  let targetMonth=month,period=LunarMonth.fromYm(year,targetMonth);if(!period&&targetMonth<0){targetMonth=Math.abs(targetMonth);period=LunarMonth.fromYm(year,targetMonth);}if(!period)throw Error('目标农历月份不存在');const lunar=Lunar.fromYmd(year,targetMonth,Math.min(day,period.getDayCount())),solar=lunar.getSolar();return date(solar.getYear(),solar.getMonth(),solar.getDay());
}
function lunarNext(value,now){const year=Lunar.fromDate(new Date(now)).getYear();let target=lunarDate(year,value.month,value.dayOfMonth);if(dayjs(target).isBefore(dayjs(now).startOf('day')))target=lunarDate(year+1,value.month,value.dayOfMonth);return target;}
export function viewBirthday(value,now=Date.now()){
  validateBirthday(value);if(!Number.isFinite(now))throw Error('当前时间无效');
  try{const target=value.dateType===1?lunarNext(value,now):solarNext(value,now),today=dayjs(target).isSame(dayjs(now),'day'),lunar=Lunar.fromDate(target);return{...copy(value),dateTime:target.toISOString(),solarText:dayjs(target).format('YYYY年MM月DD日'),lunarText:value.dateType===1?lunar.toString().split('年')[1]:'',days:today?0:Math.ceil(dayjs(target).diff(dayjs(now),'day',true)),today,error:''};}
  catch(error){return{...copy(value),dateTime:null,solarText:'日期未确定',lunarText:'',days:null,today:false,error:String(error?.message||error)};}
}
export function ordered(document,now=Date.now()){return document.birthdays.map(value=>viewBirthday(value,now)).sort((a,b)=>a.sortOrder-b.sortOrder||(a.days??Infinity)-(b.days??Infinity));}
export function birthdayDraft(value){const source=sourceDate(value),lunar=Lunar.fromDate(source);return{id:value.id,name:value.name,dateType:value.dateType,date:dayjs(source).format('YYYY-MM-DD'),year:lunar.getYear(),month:lunar.getMonth(),dayOfMonth:lunar.getDay(),introduction:value.introduction};}
export function newDraft(now=Date.now()){const old=dayjs(now).subtract(10,'years').toDate(),lunar=Lunar.fromDate(old);return{name:'',dateType:0,date:dayjs(old).format('YYYY-MM-DD'),year:lunar.getYear(),month:lunar.getMonth(),dayOfMonth:lunar.getDay(),introduction:''};}
export function birthdayFromDraft(draft,previous,now=Date.now(),id=()=>crypto.randomUUID()){
  let fields;if(draft.dateType===0){if(typeof draft.date!=='string'||!/^\d{4}-\d\d-\d\d$/.test(draft.date))throw Error('出生日期格式无效');const[year,month,dayOfMonth]=draft.date.split('-').map(Number);fields={year,month,dayOfMonth};}else fields={year:Number(draft.year),month:Number(draft.month),dayOfMonth:Number(draft.dayOfMonth)};
  const value={...(previous||{}),id:previous?.id||id(),name:String(draft.name??'').trim(),dateType:draft.dateType,...fields,introduction:previous?.introduction??draft.introduction??'',sortOrder:previous?.sortOrder??0,createdAt:previous?.createdAt??now,updatedAt:now};return copy(validateBirthday(value));
}
export function convertDraft(draft,type){if(![0,1].includes(type))throw Error('日期类型无效');const value=birthdayFromDraft({...draft,name:draft.name||'日期转换'},null,0,()=> 'draft');return{...birthdayDraft(value),name:draft.name,id:draft.id,dateType:type};}
export function reorder(document,from,to,now=Date.now()){
  const valid=validateDocument(document),byId=new Map(valid.birthdays.map(item=>[item.id,item])),values=ordered(valid,now).map(item=>byId.get(item.id));if(from===to)return valid;const index=values.findIndex(item=>item.id===from);if(index<0||to&&!byId.has(to))throw Error('拖动生日不存在');const value=values.splice(index,1)[0];values.splice(to?values.findIndex(item=>item.id===to):values.length,0,value);valid.birthdays=values.map((item,index)=>({...item,sortOrder:index}));return validateDocument(valid);
}
export function createStore(initial,persist,{preview=false,changed=()=>{}}={}){
  let value=validateDocument(initial),tail=Promise.resolve(),latest=tail,disposed=false;
  function update(change){const job=tail.then(async()=>{if(disposed)throw Error('生日视图已关闭');if(preview)throw Error('预览不保存生日');const next=copy(value),result=change(next);if(result&&typeof result.then==='function')throw Error('不接受异步修改');const valid=validateDocument(next);await persist(valid);if(!disposed){value=valid;changed(copy(value));}return copy(valid);});latest=job;tail=job.catch(()=>{});return job;}
  return{get:()=>copy(value),update,flush:()=>latest,dispose(){disposed=true;}};
}
