// 只提取服务器组件实际使用的原图标和环形画布，不引入整个监控管理应用。
import{readFileSync,writeFileSync,readdirSync,mkdirSync,existsSync}from'node:fs';import{join,dirname}from'node:path';import{createHash}from'node:crypto';import{spawnSync}from'node:child_process';
const source='artifacts/widgetjs-upstream/monitor',target='components/widgetjs/server-source',revision='e8cba77deb4ba7beb1e35939601b912bbe6aadab';
const head=spawnSync('git',['-C',source,'rev-parse','HEAD'],{encoding:'utf8'});if(head.status!==0||head.stdout.trim()!==revision)throw new Error('服务器组件源码版本不符');
if(spawnSync('git',['-C',source,'diff','--quiet',revision,'--','src'],{encoding:'utf8'}).status!==0)throw new Error('固定源码已有改动，停止提取');
const records=[];function copy(path,destination){const bytes=readFileSync(join(source,path));mkdirSync(dirname(join(target,destination)),{recursive:true});if(!existsSync(join(target,destination)))writeFileSync(join(target,destination),bytes,{flag:'wx'});records.push({source:path,target:destination,sha256:createHash('sha256').update(bytes).digest('hex')});}
for(const group of ['cloud','os'])for(const entry of readdirSync(join(source,'src/assets',group)))if(entry.endsWith('.png'))copy(join('src/assets',group,entry),join('assets',group,entry));
copy('src/components/CircleProgress.vue','CircleProgress.vue');
writeFileSync(join(target,'SOURCE_FILES.json'),JSON.stringify({repository:'rtugeek/monitor',revision,files:records},null,2)+'\n');console.log(JSON.stringify({revision,files:records.length}));
