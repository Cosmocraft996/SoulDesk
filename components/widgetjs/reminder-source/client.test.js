import test from 'node:test';
import assert from 'node:assert/strict';
const mod=await import('./client.js').catch(()=>null);
test('提醒设置默认关闭；失败保留草稿，结果未知时仅以原请求重试',async()=>{
  assert.ok(mod,'提醒设置客户端尚未实现');let fail=true;const calls=[];
  const host={preview:false,async request(method,args){calls.push([method,args]);if(method.endsWith('.get'))return {revision:0,config:mod.defaults('sit'),runtime:{}};if(fail)throw Error('宿主请求超时');return {revision:1,config:args.config,runtime:{}};}};
  const editor=mod.createSettings(host,'sit');await editor.load();assert.equal(editor.state.document.config.enabled,false);
  editor.edit({...editor.state.draft,enabled:true,sitInterval:1});await editor.save();assert.equal(editor.state.draft.sitInterval,1);assert.equal(editor.state.document.config.enabled,false);assert.equal(editor.state.uncertain,true);
  assert.throws(()=>editor.edit({...editor.state.draft,sitInterval:2}),/结果未确定/);fail=false;await editor.save();assert.deepEqual(calls[1],calls[2]);assert.equal(editor.state.document.config.enabled,true);assert.equal(editor.state.uncertain,false);
});
test('晚到的状态不覆盖已保存版本；预览不调用后台',async()=>{
  assert.ok(mod);let finish;const editor=mod.createSettings({preview:false,request(method,args){if(method.endsWith('.get'))return new Promise(resolve=>finish=resolve);return Promise.resolve({revision:1,config:args.config,runtime:{}});}},'island');
  const pending=editor.refresh();editor.edit({enabled:true});await editor.save();finish({revision:0,config:{enabled:false},runtime:{}});await pending;assert.equal(editor.state.document.revision,1);assert.equal(editor.state.draft.enabled,true);
  const preview=mod.createSettings({preview:true,request(){assert.fail('预览不能访问后台');}},'phone');await preview.load();await preview.save();assert.equal(preview.state.document.config.enabled,false);assert.equal(preview.state.document.config.rules.length,4);
});
test('歌词以音频实际时刻选择，倒计时不为负且百分比有界',()=>{
  assert.ok(mod);const lines=mod.parseLyrics('[00:00.90]四\n[00:01.80]三\n[bad]忽略');assert.equal(mod.lyricAt(lines,.3,'等待'),'等待');assert.equal(mod.lyricAt(lines,1,'等待'),'四');assert.equal(mod.lyricAt(lines,2,'等待'),'三');assert.equal(mod.clockText(-1),'00:00:00');assert.equal(mod.clockText(61234),'01:01:23');assert.equal(mod.progress(200,100),100);assert.equal(mod.progress(-20,100),0);
});
test('音频播放被拦截如实反馈；关闭后晚到的播放成功不能重新启音',async()=>{
  assert.ok(mod);let resolvePlay;const reports=[];const voice={currentTime:0,paused:0,play(){return Promise.reject(Object.assign(Error('blocked'),{name:'NotAllowedError'}));},pause(){this.paused++;}};
  const audio=mod.createPlayback({voice,hangup:{pause(){},play(){return Promise.resolve();}}},state=>reports.push(state));await audio.start();assert.equal(audio.state.playing,false);assert.match(audio.state.error,/接听/);assert.deepEqual(reports,['blocked']);
  voice.play=()=>new Promise(resolve=>resolvePlay=resolve);const pending=audio.start();audio.stop();resolvePlay();await pending;assert.equal(audio.state.playing,false);assert.ok(voice.paused>=2);assert.deepEqual(reports,['blocked']);
});
