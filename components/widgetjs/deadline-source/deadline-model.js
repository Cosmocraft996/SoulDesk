import dayjs from 'dayjs';
export const defaults={title:'Deadline',startTime:'',endTime:'',primaryColor:'#BE002A',background:'rgba(0,0,0,0.5)',radius:22};
function timestamp(value){if(typeof value!=='string'||!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value)||!Number.isFinite(Date.parse(value))||new Date(value).toISOString()!==value)throw new Error('期限日期无效');return Date.parse(value);}
function color(value){return typeof value==='string'&&value.length<=80&&(/^(?:#[a-f\d]{3,4}|#[a-f\d]{6}|#[a-f\d]{8}|transparent)$/i.test(value)||/^(?:rgb|hsl)a?\([\d.% ,/+-]+\)$/i.test(value));}
export function validateSettings(raw){
  const value={...defaults,...raw};const start=timestamp(value.startTime),end=timestamp(value.endTime);
  if(end-start<60000)throw new Error('最后期限必须晚于开始时间至少一分钟');
  if(typeof value.title!=='string'||value.title.length>120)throw new Error('标题不能超过120字');
  if(!color(value.primaryColor)||!color(value.background))throw new Error('颜色格式无效');
  if(!Number.isInteger(value.radius)||value.radius<0||value.radius>50)throw new Error('圆角须为0～50的整数');
  return value;
}
export function initialSettings(raw,now=Date.now()){
  const value={...defaults,...raw};
  if(value.startTime===''&&value.endTime===''){
    const start=new Date(now),end=new Date(now);start.setDate(start.getDate()-1);end.setDate(end.getDate()+1);
    value.startTime=start.toISOString();value.endTime=end.toISOString();
  }
  return validateSettings(value);
}
export async function loadSettings(host,now=Date.now()){
  const raw=await host.request('settings.get'),value=initialSettings(raw,now);
  if(!host.preview&&raw.startTime===''&&raw.endTime==='')return validateSettings(await host.request('settings.update',value));
  return value;
}
export function deadline(value,now=Date.now()){
  const settings=validateSettings(value);if(!Number.isFinite(now))throw new Error('当前时间无效');
  const start=dayjs(settings.startTime),end=dayjs(settings.endTime);
  // 保留原整分钟口径；显式传入当前时刻，避免非响应时间被永久缓存。
  const ratio=Math.min(1,Math.max(0,dayjs(now).diff(start,'minutes')/end.diff(start,'minutes')));
  return{remainingDays:end.diff(dayjs(now),'day'),percent:Math.round(ratio*100),fillX:Math.round(ratio*100-100),deathX:Math.round(110+ratio*410)};
}
export function localDay(value){const date=new Date(timestamp(value));return date.getFullYear()+'-'+String(date.getMonth()+1).padStart(2,'0')+'-'+String(date.getDate()).padStart(2,'0');}
function fromLocalDay(text){
  if(typeof text!=='string'||!/^\d{4}-\d\d-\d\d$/.test(text))throw new Error('请选择有效日期');
  const[y,m,d]=text.split('-').map(Number),date=new Date(0);date.setHours(0,0,0,0);date.setFullYear(y,m-1,d);
  if(y<1||date.getFullYear()!==y||date.getMonth()!==m-1||date.getDate()!==d)throw new Error('请选择有效日期');
  return date.toISOString();
}
export function settingsFromDraft(applied,draft){
  return validateSettings({...applied,title:draft.title,primaryColor:draft.primaryColor,background:draft.background,radius:Number(draft.radius),startTime:draft.startDate===localDay(applied.startTime)?applied.startTime:fromLocalDay(draft.startDate),endTime:draft.endDate===localDay(applied.endTime)?applied.endTime:fromLocalDay(draft.endDate)});
}
