import test from'node:test';import assert from'node:assert/strict';import{existsSync}from'node:fs';
const path=new URL('./birthday-model.js',import.meta.url);async function model(){assert.ok(existsSync(path),'生日本地模型尚未实现');return import(path);}
const date=(y,m,d,h=0)=>new Date(y,m-1,d,h),birth=(extra={})=>({id:'a',name:'小李',dateType:0,year:1990,month:11,dayOfMonth:15,introduction:'保留字段',sortOrder:0,createdAt:1,updatedAt:1,...extra});
test('公历生日当天为零且只高亮当天，跨日重新计算，不缓存过期的今天标记',async()=>{
  const m=await model(),value=birth();const today=m.viewBirthday(value,date(2026,11,15,12).getTime());assert.equal(today.today,true);assert.equal(today.days,0);assert.equal(today.solarText,'2026年11月15日');const later=m.viewBirthday(value,date(2026,11,16,0).getTime());assert.equal(later.today,false);assert.equal(later.solarText,'2027年11月15日');assert.equal(later.days,364);assert.equal(m.viewBirthday(birth({month:1,dayOfMonth:1}),date(2026,12,31,12).getTime()).days,1);
});
test('二月二十九沿原非闰年三月一日口径，进入闰年恢复二月二十九',async()=>{
  const m=await model(),value=birth({year:2000,month:2,dayOfMonth:29});assert.equal(m.viewBirthday(value,date(2026,2,28,12).getTime()).solarText,'2026年03月01日');assert.equal(m.viewBirthday(value,date(2027,12,31,12).getTime()).solarText,'2028年02月29日');assert.equal(m.viewBirthday(value,date(2028,2,29,12).getTime()).today,true);
});
test('农历新年前后按真实农历年找下一次生日，大小月按源规则截到该月末日',async()=>{
  const m=await model(),value=birth({dateType:1,year:2000,month:12,dayOfMonth:15});const next=m.viewBirthday(value,date(2027,1,15,12).getTime());assert.equal(next.solarText,'2027年01月22日');assert.equal(next.lunarText,'腊月十五');assert.equal(next.days,7);
  const smaller=m.viewBirthday(birth({dateType:1,year:2020,month:4,dayOfMonth:30}),date(2026,5,1,12).getTime());assert.equal(smaller.error,'');assert.equal(smaller.lunarText,'四月廿九');assert.ok(smaller.days>0);
});
test('闰月生日在目标年无同名闰月时按普通同月计算，有闰月时仍保留闰月',async()=>{
  const m=await model(),value=birth({dateType:1,year:2020,month:-4,dayOfMonth:1});
  const fallback=m.viewBirthday(value,date(2026,5,1).getTime());assert.equal(fallback.error,'');assert.equal(fallback.solarText,'2026年05月17日');assert.equal(fallback.lunarText,'四月初一');assert.equal(fallback.month,-4,'出生闰月身份不得被改写');
  const leap=m.viewBirthday(value,date(2020,5,1).getTime());assert.equal(leap.solarText,'2020年05月23日');assert.equal(leap.lunarText,'闰四月初一');assert.equal(leap.error,'');
  const next=m.viewBirthday(value,date(2026,5,18).getTime());assert.equal(next.solarText,'2027年05月06日');assert.equal(next.error,'');
  const today=m.viewBirthday(value,date(2026,5,17,12).getTime());assert.equal(today.today,true);assert.equal(today.days,0);
});

test('闰月转普通月份后仍沿原小月末日规则，并跨农历新年计算',async()=>{
  const m=await model(),value=birth({dateType:1,year:1952,month:-5,dayOfMonth:30});
  const result=m.viewBirthday(value,date(2026,3,1).getTime());assert.equal(result.error,'');assert.equal(result.solarText,'2026年07月13日');assert.equal(result.lunarText,'五月廿九');assert.equal(result.month,-5);assert.equal(result.dayOfMonth,30);
  const early=m.viewBirthday(birth({dateType:1,year:2020,month:-4,dayOfMonth:1}),date(2027,1,15).getTime());assert.equal(early.solarText,'2027年05月06日');assert.equal(early.error,'');
});

test('公历与农历编辑互转保持同一出生日期，名字和不可见介绍字段不丢失',async()=>{
  const m=await model(),value=birth({year:2000,month:2,dayOfMonth:5}),draft=m.birthdayDraft(value),lunar=m.convertDraft(draft,1);assert.equal(lunar.year,2000);assert.equal(lunar.month,1);assert.equal(lunar.dayOfMonth,1);const saved=m.birthdayFromDraft(lunar,value,2,()=> 'unused');assert.equal(saved.id,'a');assert.equal(saved.dateType,1);assert.equal(saved.introduction,'保留字段');const solar=m.convertDraft(m.birthdayDraft(saved),0);assert.equal(solar.date,'2000-02-05');assert.equal(m.birthdayFromDraft(solar,saved,3).dateType,0);
});
test('损坏版本、重复标识、非法生日/名字/主题及超限存储被拒绝，空列表无伪造联系人',async()=>{
  const m=await model();assert.deepEqual(m.restoreDocument({}),{version:1,birthdays:[]});assert.throws(()=>m.restoreDocument({version:9}),/存储/);assert.throws(()=>m.validateDocument({version:1,birthdays:[birth(),birth()]}),/重复/);assert.throws(()=>m.validateDocument({version:1,birthdays:[birth({year:1999,month:2,dayOfMonth:29})]}),/日期/);assert.throws(()=>m.birthdayFromDraft({name:' ',dateType:0,date:'2000-01-01'},null,1,()=> 'x'),/名字|姓名/);assert.throws(()=>m.validateDocument({version:1,birthdays:[birth()]},10),/上限/);assert.throws(()=>m.validateSettings({background:'url(https://example.com)'}),/颜色/);
});
test('按生日临近与手动顺序排序，串行保存失败保留旧值且可重试，预览不写入',async()=>{
  const m=await model(),doc={version:1,birthdays:[birth({id:'far',month:12,dayOfMonth:31}),birth({id:'near',month:11,dayOfMonth:16})]};assert.deepEqual(m.ordered(doc,date(2026,11,15).getTime()).map(x=>x.id),['near','far']);assert.deepEqual(m.reorder(doc,'far','near',date(2026,11,15).getTime()).birthdays.map(x=>[x.id,x.sortOrder]),[['far',0],['near',1]]);
  let fail=true;const store=m.createStore(doc,async()=>{if(fail)throw Error('磁盘只读');});await assert.rejects(store.update(next=>{next.birthdays[0].name='失败';}),/磁盘只读/);assert.equal(store.get().birthdays[0].name,'小李');fail=false;await store.update(next=>{next.birthdays[0].name='成功';});assert.equal(store.get().birthdays[0].name,'成功');store.dispose();await assert.rejects(store.update(()=>{}),/关闭/);const preview=m.createStore(doc,async()=>{throw Error('不应写入');},{preview:true});await assert.rejects(preview.update(()=>{}),/预览/);
});

test('切换共享库前可等待全部已提交保存，不能提前重载丢失写入',async()=>{const m=await model();let finish,start,flushed=false;const started=new Promise(resolve=>{start=resolve;});const initial={version:1,birthdays:[]};const store=m.createStore(initial,()=>{start();return new Promise(resolve=>{finish=resolve;});});const updating=store.update(()=>{});await started;const waiting=store.flush().then(()=>{flushed=true;});await Promise.resolve();assert.equal(flushed,false);finish();await updating;await waiting;assert.equal(flushed,true);store.dispose();});

test('上次保存失败时flush继续拒绝共享库切换，不能吞掉原错误',async()=>{const m=await model();const initial={version:1,birthdays:[]};const store=m.createStore(initial,async()=>{throw Error('磁盘只读');});await assert.rejects(store.update(()=>{}),/磁盘只读/);await assert.rejects(store.flush(),/磁盘只读/);assert.deepEqual(store.get(),initial);store.dispose();});
