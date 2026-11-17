// 提取已审计的固定部署资源和原提示词，不执行上游JavaScript。
import{readFileSync,writeFileSync,mkdirSync,existsSync}from'node:fs';
import{basename,dirname,join}from'node:path';import{createHash}from'node:crypto';import ts from'typescript';
const audit=JSON.parse(readFileSync('artifacts/widgetjs-ai-job-complete-source-audit.json','utf8'));
const target='components/widgetjs/job-source',revision='58806a7f80ab5389b49087dd66498bdd7c1982c2578cb199ead31b4e27d3d11f';
if(audit.rootSha256!==revision||audit.failures.length||audit.files.length!==31)throw Error('固定来源清单不完整');
const files=[];for(const item of audit.files){const bytes=readFileSync(item.file),sha256=createHash('sha256').update(bytes).digest('hex');if(bytes.length!==item.bytes||sha256!==item.sha256)throw Error('固定来源字节变化:'+item.file);const path='original/assets/'+basename(item.file),out=join(target,path);mkdirSync(dirname(out),{recursive:true});if(existsSync(out)){if(!readFileSync(out).equals(bytes))throw Error('既有来源不同，未覆盖:'+path);}else writeFileSync(out,bytes,{flag:'wx'});files.push({url:item.url,path,bytes:bytes.length,sha256});}
const entry=files.find(f=>f.path.endsWith('/index-Bfkv8pFi.js'));
const source=ts.createSourceFile(entry.path,readFileSync(join(target,entry.path),'utf8'),ts.ScriptTarget.Latest,true,ts.ScriptKind.JS),prompts={};
function literal(n){
 if(ts.isStringLiteral(n)||ts.isNoSubstitutionTemplateLiteral(n))return n.text;
 if(ts.isNumericLiteral(n))return Number(n.text);
 if(ts.isArrayLiteralExpression(n))return n.elements.map(literal);
 if(ts.isObjectLiteralExpression(n))return Object.fromEntries(n.properties.map(p=>{if(!ts.isPropertyAssignment(p))throw Error('非静态属性');return[p.name.text,literal(p.initializer)];}));
 if(ts.isPropertyAccessExpression(n)&&ts.isIdentifier(n.expression)&&n.expression.text==='vo'&&Object.hasOwn(prompts,n.name.text))return prompts[n.name.text];
 if(ts.isCallExpression(n)&&n.expression.getText(source)==='JSON.stringify')return JSON.stringify(literal(n.arguments[0]));
 if(ts.isTemplateExpression(n))return n.head.text+n.templateSpans.map(s=>String(literal(s.expression))+s.literal.text).join('');
 throw Error('提示词不是允许的静态表达式:'+n.getText(source).slice(0,80));
}
function visit(n){if(ts.isCallExpression(n)&&n.arguments[0]?.getText(source)==='vo'&&n.arguments[1]&&ts.isStringLiteral(n.arguments[1]))prompts[n.arguments[1].text]=literal(n.arguments[2]);ts.forEachChild(n,visit);}visit(source);
if(Object.keys(prompts).length!==7||!prompts.jobAnalysis||!prompts.interviewer)throw Error('原提示词数量不符');
writeFileSync(join(target,'prompts.json'),JSON.stringify(prompts,null,2)+'\n');
writeFileSync(join(target,'SOURCE_FILES.json'),JSON.stringify({entry:'https://widgetjs.cn/ai/job/',revision,versionType:'固定公开部署入口SHA-256（未取得Git源码提交）',license:'公开部署未附项目LICENSE；原资源及其内嵌第三方声明完整保留，按用户已确认的自用范围移植',files},null,2)+'\n');
console.log(JSON.stringify({revision,files:files.length,bytes:files.reduce((n,f)=>n+f.bytes,0),prompts:Object.keys(prompts)}));
