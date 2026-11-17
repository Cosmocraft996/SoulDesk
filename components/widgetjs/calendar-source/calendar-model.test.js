import test from'node:test';import assert from'node:assert/strict';import{defaults,endpoint,monthView,navigate,parseAlmanac,Solar,todayKey,validateSettings,bottomLabel}from'./calendar-model.js';
const envelope=data=>({source:endpoint,fetchedAt:1,data:{code:0,data}}),date=(year,month,day)=>new Date(year,month-1,day,12);
test('原历法分周保留周日/周一、闰日、相邻月和跨年；今天与周数不混淆年份',()=>{
  const monday=monthView(date(2026,11,15),1,'2026-11-15'),sunday=monthView(date(2026,11,15),0,'2026-11-15');assert.equal(monday.weeks.length,6);assert.equal(monday.weeks[0][0].key,'2026-10-26');assert.equal(monday.weeks.at(-1).at(-1).key,'2026-12-06');assert.equal(sunday.weeks.length,5);assert.equal(sunday.weeks[0][0].key,'2026-11-01');assert.equal(monday.weeks.flat().filter(x=>x.today).length,1);assert.equal(monday.isoWeek,46);
  for(const[year,days]of[[2024,29],[2026,28]])assert.equal(monthView(date(year,2,15),1,`${year}-02-15`).weeks.flat().filter(x=>!x.outside).length,days);
  const december=monthView(date(2026,12,31),1,'2026-12-31');assert.ok(december.weeks.flat().some(x=>x.key.startsWith('2027-01')&&x.outside));assert.equal(monthView(date(2025,11,15),1,'2026-11-15').isoWeek,null);
});
test('日期底标沿原短公历节日、源term、农历日顺序，不添加其他节假日数据',()=>{
  const festival=Solar.fromYmd(2026,11,1);assert.equal(bottomLabel(festival,{term:'来源文字'}),festival.getFestivals()[0]);
  const ordinary=Solar.fromYmd(2026,11,15);assert.equal(bottomLabel(ordinary,{term:'源标记'}),'源标记');assert.equal(bottomLabel(ordinary),ordinary.getLunar().getDayInChinese());
  assert.equal(bottomLabel(Solar.fromYmd(2026,2,17)),'初一');
});
test('休/班只取原源status；日期范围和原宜忌保存，周末描述不推导休班',()=>{
  const parsed=parseAlmanac(envelope([{year:2026,month:11,dayOfMonth:15,status:1,suit:'出行.沐浴',avoid:'无',term:'来源节日',date:'2026-11-15'},{year:'2026',month:'11',dayOfMonth:'16',status:'2'},{year:2026,month:11,dayOfMonth:21,statusDesc:'周末'}]));assert.equal(parsed.records.size,3);assert.equal(parsed.firstDate,'2026-11-15');assert.equal(parsed.lastDate,'2026-11-21');assert.equal(parsed.records.get('2026-11-16').status,2);assert.equal(parsed.records.get('2026-11-21').status,0);
  const view=monthView(date(2026,11,15),1,'2026-11-15',parsed.records);assert.equal(view.missing,27);assert.equal(view.weeks.flat().find(x=>x.key==='2026-11-15').status,1);assert.equal(parsed.records.get('2026-11-15').suit,'出行.沐浴');
});
test('原接口错误、数据过量、重复/不存在/冲突日期与未知休班状态明确失败',()=>{
  const row={year:2026,month:11,dayOfMonth:15};for(const value of [{...envelope([]),source:'https://other.test'},envelope(Array(5001).fill(row)),envelope([row,row]),envelope([{...row,date:'2026-11-16'}]),envelope([{year:2026,month:2,dayOfMonth:30}]),envelope([{...row,status:3}]),envelope([{...row,suit:'a'.repeat(4097)}]),{...envelope([]),data:{code:1,data:[]}}])assert.throws(()=>parseAlmanac(value));
  assert.equal(parseAlmanac(envelope([])).records.size,0);assert.equal(parseAlmanac(envelope([{year:1582,month:10,dayOfMonth:31}])).records.size,1);
});
test('设置与相邻月导航保留源范围，跨年和月底不溢出',()=>{
  assert.deepEqual(validateSettings({}),defaults);for(const next of[{firstDayOfWeek:2},{fontSize:9},{fontSize:41},{radius:51},{primaryColor:'url(https://example.test)'}])assert.throws(()=>validateSettings(next));
  const next=navigate(date(2026,12,31),1);assert.equal(next.getFullYear(),2027);assert.equal(next.getMonth(),0);assert.equal(navigate(date(2026,1,31),1).getDate(),28);assert.throws(()=>navigate(date(2026,1,1),12));assert.throws(()=>todayKey(NaN));
});
