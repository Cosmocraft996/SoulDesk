// 只从固定Git对象提取源码/音频/图片；不执行上游脚本，不修改仓库工作树或全局代理。
import{spawnSync}from'node:child_process';import{readFileSync,writeFileSync,mkdirSync,existsSync}from'node:fs';import{dirname,join}from'node:path';import{createHash}from'node:crypto';
const repository='artifacts/widgetjs-upstream/common-history',revision='714802e66336133396ab55463eb5a26d08fc0788',target='components/widgetjs/reminder-source';
const roots=['src/widgets/dynamic-island','src/widgets/sit-reminder','src/widgets/phone-reminder','LICENSE','public/audio/ringtone.m4a','public/audio/hangup.m4a','public/audio/voice/squidward.m4a','public/audio/voice/elvis.mp3','public/audio/voice/mrs_pan.m4a','public/audio/voice/jackie_chan.wav','public/images/zhangyuge.jpg','public/images/avatar/elvis.png','public/images/avatar/mrs_pan.jpg','public/images/avatar/jackie_chan.jpg'];
const listing=spawnSync('git',['-c','http.proxy=','-C',repository,'ls-tree','-r','--name-only',revision,...roots],{encoding:'utf8'});if(listing.status!==0)throw Error('无法枚举固定提醒组件来源');const files=[];
for(const path of listing.stdout.trim().split('\n')){
  const result=spawnSync('git',['-c','http.proxy=','-C',repository,'show',`${revision}:${path}`],{maxBuffer:16*1024*1024});if(result.status!==0)throw Error('读取固定资源失败：'+path);
  const relative=path==='LICENSE'?'LICENSE.txt':'original/'+path,out=join(target,relative);mkdirSync(dirname(out),{recursive:true});if(existsSync(out)){if(!readFileSync(out).equals(result.stdout))throw Error('已有原始来源字节不同，未覆盖：'+out);}else writeFileSync(out,result.stdout,{flag:'wx'});
  files.push({source:path,path:relative,bytes:result.stdout.length,sha256:createHash('sha256').update(result.stdout).digest('hex')});
}
writeFileSync(join(target,'SOURCE_FILES.json'),JSON.stringify({repository:'https://github.com/widget-js/widgets',revision,files,scope:'灵动通知、久坐提醒及来电提醒的固定源码与原图片/音频，仅提取不执行'},null,2)+'\n');console.log(JSON.stringify({revision,files:files.length,bytes:files.reduce((n,f)=>n+f.bytes,0)}));
