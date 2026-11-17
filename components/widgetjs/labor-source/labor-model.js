const fields=['startHour','startMinute','endHour','endMinute'];
export function shift(settings,now=Date.now()){
  if(!Number.isSafeInteger(now)||fields.some((key,index)=>!Number.isInteger(settings[key])||settings[key]<0||settings[key]>(index%2?59:23)))throw new Error('上下班时间区间无效');
  const startMinute=settings.startHour*60+settings.startMinute,endMinute=settings.endHour*60+settings.endMinute;if(startMinute===endMinute)throw new Error('上下班时间不能相同');
  const date=new Date(now),start=new Date(now),end=new Date(now),currentMinute=date.getHours()*60+date.getMinutes();start.setHours(settings.startHour,settings.startMinute,0,0);end.setHours(settings.endHour,settings.endMinute,0,0);
  if(endMinute<startMinute){if(currentMinute<startMinute)start.setDate(start.getDate()-1);else end.setDate(end.getDate()+1);}
  if(end.getTime()<=start.getTime())throw new Error('上下班区间无法计算');
  const percent=Math.min(100,Math.max(0,Math.floor((now-start.getTime())/(end.getTime()-start.getTime())*1000)/10));
  return {startAt:start.getTime(),endAt:end.getTime(),percent,remainingSeconds:Math.max(0,Math.ceil((end.getTime()-now)/1000)),isoWeekday:date.getDay()||7};
}
export function timeline(percent,weekday=1){
  const days=['face_with_spiral_eyes','face_with_steam_from_nose','knocked-out_face','face_with_rolling_eyes','face_holding_back_tears','face_holding_back_tears','face_holding_back_tears'];
  let image=days[weekday-1]||days[0],title='-1s',duration=1;
  if(percent>=100){image='partying_face';title='❤';duration=3;}else if(percent>=95){image='star-struck';title='快';duration=.2;}else if(percent>=55.7){image='smiling_face_with_sunglasses';title='冲';}else if(percent>=49.1){image='sleeping_face';title='困';}else if(percent>=44.5){image='sleepy_face';title='困';}else if(percent>=33.3){image='yawning_face';title='困';}
  return {image:image+'_3d.png',title,duration};
}
export function view(snapshot,now=Date.now()){
  const invalid=!snapshot||!Number.isSafeInteger(snapshot.sampledAt)||snapshot.sampledAt>now+5000||now-snapshot.sampledAt>10000||!Number.isSafeInteger(snapshot.startAt)||!Number.isSafeInteger(snapshot.endAt)||snapshot.endAt<=snapshot.startAt;
  if(invalid)return {percent:null,remainingSeconds:null,emoji:timeline(0,1),status:'时间数据尚未就绪或已过期'};
  const percent=Math.min(100,Math.max(0,Math.floor((now-snapshot.startAt)/(snapshot.endAt-snapshot.startAt)*1000)/10));
  return {percent,remainingSeconds:Math.max(0,Math.ceil((snapshot.endAt-now)/1000)),emoji:timeline(percent,snapshot.isoWeekday),status:snapshot.lastError||''};
}
