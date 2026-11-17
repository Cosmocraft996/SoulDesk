import{S as Solar,c as SolarMonth}from'./original/index-foMHgrIw.js';
import dayjs from'dayjs';import isoWeek from'dayjs/plugin/isoWeek.js';dayjs.extend(isoWeek);
export{Solar,SolarMonth};
export const endpoint='https://itime.fun/api/v2/event/public/calendar';
export const defaults={firstDayOfWeek:1,background:'rgba(0,0,0,0.5)',foreground:'#ffffff',primaryColor:'rgb(0, 149, 255)',fontSize:14,radius:22};
const color=value=>typeof value==='string'&&value.length<=80&&(/^(?:#[a-f\d]{3,4}|#[a-f\d]{6}|#[a-f\d]{8}|transparent)$/i.test(value)||/^(?:rgb|hsl)a?\([\d.% ,/+-]+\)$/i.test(value));
export function validateSettings(raw){const value={...defaults,...raw};if(![0,1].includes(value.firstDayOfWeek))throw new Error('一周开始日只能为周日或周一');if(!Number.isInteger(value.fontSize)||value.fontSize<10||value.fontSize>40)throw new Error('字号须为10～40的整数');if(!Number.isInteger(value.radius)||value.radius<0||value.radius>50)throw new Error('圆角须为0～50的整数');for(const key of ['background','foreground','primaryColor'])if(!color(value[key]))throw new Error('颜色格式无效');return value;}
export function todayKey(now=Date.now()){if(!Number.isFinite(now))throw new Error('当前日期无效');return Solar.fromDate(new Date(now)).toYmd();}
function integer(value,min,max){const number=typeof value==='string'&&/^\d+$/.test(value)?Number(value):value;if(!Number.isInteger(number)||number<min||number>max)throw new Error('黄历日期字段无效');return number;}
function text(value,max){if(value===null||value===undefined)return'';if(typeof value!=='string'||value.length>max)throw new Error('黄历文本字段无效');return value;}
export function parseAlmanac(response){
  if(response?.source!==endpoint||!Number.isFinite(response?.fetchedAt)||response.fetchedAt<0||response?.data?.code!==0||!Array.isArray(response.data.data)||response.data.data.length>5000)throw new Error('黄历来源或响应格式无效');
  const records=new Map(),monthDates=new Map();
  for(const raw of response.data.data){if(!raw||typeof raw!=='object'||Array.isArray(raw))throw new Error('黄历条目无效');const year=integer(raw.year,1,9999),month=integer(raw.month,1,12),day=integer(raw.dayOfMonth,1,31),solar=Solar.fromYmd(year,month,day),key=solar.toYmd(),monthKey=year+'-'+month;if(!monthDates.has(monthKey))monthDates.set(monthKey,new Set(SolarMonth.fromYm(year,month).getDays().map(day=>day.toYmd())));if(!monthDates.get(monthKey).has(key)||raw.date!==undefined&&raw.date!==key||records.has(key))throw new Error('黄历日期冲突或不存在');const status=raw.status===null||raw.status===undefined?0:integer(raw.status,0,2);records.set(key,{year,month,dayOfMonth:day,status,term:text(raw.term,128),suit:text(raw.suit,4096),avoid:text(raw.avoid,4096),sourceUpdatedAt:text(raw.updateTime,80)});}
  const keys=[...records.keys()].sort();return{records,fetchedAt:response.fetchedAt,source:response.source,firstDate:keys[0]||null,lastDate:keys.at(-1)||null};
}
export function bottomLabel(solar,almanac){const first=solar.getFestivals()[0];return first&&first.length<=6?first:almanac?.term||solar.getLunar().getDayInChinese();}
export function monthView(cursor,firstDayOfWeek,today,records=new Map()){
  if(![0,1].includes(firstDayOfWeek))throw new Error('一周开始日无效');const date=dayjs(cursor);if(!date.isValid()||date.year()<1||date.year()>9999)throw new Error('月份无效');
  const year=date.year(),month=date.month()+1,weeks=SolarMonth.fromYm(year,month).getWeeks(firstDayOfWeek).map(week=>week.getDays().map(solar=>{const key=solar.toYmd(),almanac=records.get(key);return{solar,key,day:solar.getDay(),outside:solar.getYear()!==year||solar.getMonth()!==month,today:key===today,label:bottomLabel(solar,almanac),status:almanac?.status||0,almanac};}));
  return{year,month,title:year+'年'+month+'月',weekdays:firstDayOfWeek===0?['日','一','二','三','四','五','六']:['一','二','三','四','五','六','日'],weeks,missing:weeks.flat().filter(day=>!day.outside&&!day.almanac).length,isoWeek:today.startsWith(year+'-'+String(month).padStart(2,'0')+'-')?dayjs(today).isoWeek():null};
}
export function navigate(cursor,delta){if(![-1,1].includes(delta))throw new Error('只支持相邻月份');const next=dayjs(cursor).add(delta,'month');if(next.year()<1||next.year()>9999)throw new Error('月份超出范围');return next.toDate();}
