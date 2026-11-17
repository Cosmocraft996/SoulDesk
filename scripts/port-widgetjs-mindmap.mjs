// 从已固定的源码副本提取本地编辑器及实际使用的界面文件，不复制账户或云API。
import {readFileSync,writeFileSync,readdirSync,mkdirSync,existsSync} from 'node:fs';
import {join,dirname,resolve,relative} from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
const source=resolve('artifacts/widgetjs-upstream/mindmap'),target=resolve('components/widgetjs/mindmap-source');
const revision='9e8ff6ace455e7930117dec3816821ada51c48ab';
const head=spawnSync('git',['-C',source,'rev-parse','HEAD'],{encoding:'utf8'});if(head.status!==0||head.stdout.trim()!==revision)throw new Error('思维导图源码提交不符');
const clean=spawnSync('git',['-C',source,'diff','--quiet',revision,'--','src','packages/mindmap/src'],{encoding:'utf8'});if(clean.status!==0)throw new Error('固定源码存在本地改动，停止提取');
const records=[];
function copy(path,out=path){
  const origin=join(source,path),destination=join(target,out);if(relative(target,destination).startsWith('..'))throw new Error('目标路径越界');
  const bytes=readFileSync(origin);records.push({source:path,target:out,sha256:createHash('sha256').update(bytes).digest('hex')});
  mkdirSync(dirname(destination),{recursive:true});if(!existsSync(destination))writeFileSync(destination,bytes,{flag:'wx'});
}
function tree(path,out){for(const entry of readdirSync(join(source,path),{withFileTypes:true})){const next=join(path,entry.name),dest=join(out,entry.name);if(entry.isDirectory())tree(next,dest);else copy(next,dest);}}
tree('packages/mindmap/src','editor');
const copied=new Set();
function ui(path){
  if(copied.has(path))return;copied.add(path);copy('src/'+path,'ui/'+path);
  const text=readFileSync(join(source,'src',path),'utf8');
  for(const match of text.matchAll(/(?:from\s*|import\s*)['"](@\/[^'"]+)['"]/g)){
    const stem=match[1].slice(2);const found=['.tsx','.ts','.jsx','.js','.css'].map(ext=>stem+ext).find(file=>existsSync(join(source,'src',file)));
    if(!found)throw new Error('未定位界面依赖：'+stem);ui(found);
  }
}
for(const name of ['sidebar','collapsible','dropdown-menu','popover','label','dialog','button','input','separator','empty'])ui('components/ui/'+name+'.tsx');
for(const file of ['TextType.jsx','TextType.css','TextType.d.ts'])copy('src/components/'+file,'ui/components/'+file);
copy('src/assets/logo.png','logo.png');copy('src/index.css','base.css');
writeFileSync(join(target,'SOURCE_FILES.json'),JSON.stringify({repository:'rtugeek/mindmap',revision,files:records},null,2)+'\n');
console.log(JSON.stringify({revision,files:records.length}));
