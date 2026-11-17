// 由固定部署字节派生原Vue渲染与CSS；不执行上游入口、Electron桥、数据库或OpenAI客户端。
import ts from 'typescript';
import{readFileSync,writeFileSync,mkdirSync}from'node:fs';
import{createHash}from'node:crypto';
const root='components/widgetjs/job-source',source=root+'/original/assets/',out=root+'/derived/';
const provenance=JSON.parse(readFileSync(root+'/SOURCE_FILES.json','utf8'));
mkdirSync(out,{recursive:true});
const selected=new Set(['AiConfigForm','SettingsDialog','JobSearchConfig','LogPanel','BossWebView','JobView','JobCard','PaginationScrollbar','PieChart','KnowledgePieChart','JobAnalysisView','WindowTitleBar','Window','Markdown','ChatBox','ChatInput','InterviewSection','MarkdownSection','MarkdownMap','MindMapSection','ContextMenu','KnowledgeView']);
const custom=new Set(['WindowTitleBar','Window','BossWebView','Markdown','MarkdownMap','ContextMenu']);
const changed=[],notices=[];
function names(node,sf){const values=[];function walk(n){if(ts.isIdentifier(n))values.push(n.text);else if(ts.isObjectBindingPattern(n)||ts.isArrayBindingPattern(n))for(const x of n.elements)if(ts.isBindingElement(x))walk(x.name);}for(const s of node.body.statements){if(ts.isVariableStatement(s))for(const d of s.declarationList.declarations)walk(d.name);if(ts.isFunctionDeclaration(s))values.push(s.name.text);}return [...new Set(values)];}
for(const file of provenance.files){
 const name=file.path.split('/').at(-1),bytes=readFileSync(root+'/'+file.path);
 if(bytes.length!==file.bytes||createHash('sha256').update(bytes).digest('hex')!==file.sha256)throw Error('固定来源字节发生变化：'+name);
 if(!name.endsWith('.js')){writeFileSync(out+name,bytes);continue;}
 const raw=bytes.toString('utf8'),sf=ts.createSourceFile(name,raw,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS),edits=[];let usesUi=false;
 const comments=new Map();function commentRanges(node){for(const range of[...(ts.getLeadingCommentRanges(raw,node.pos)??[]),...(ts.getTrailingCommentRanges(raw,node.end)??[])])if(range.kind===ts.SyntaxKind.MultiLineCommentTrivia)comments.set(range.pos,raw.slice(range.pos,range.end));ts.forEachChild(node,commentRanges);}commentRanges(sf);
 for(const[,comment]of[...comments].sort(([a],[b])=>a-b))if(/copyright|@license|licensed|MIT License|Apache|BSD/i.test(comment))notices.push(name+'\n'+comment);
 const edit=(node,text)=>edits.push([node.getStart(sf),node.end,text]);
 function walk(node){
  if(ts.isObjectLiteralExpression(node)){
   const label=node.properties.find(p=>p.name?.getText(sf)==='__name');
   const component=ts.isPropertyAssignment(label??{})?label.initializer.text:null;
   const setup=node.properties.find(p=>p.name?.getText(sf)==='setup');
   if(selected.has(component)&&setup?.body){
    const props=setup.parameters[0]?.name.getText(sf)??'__props';const ctx=setup.parameters[1]?.name;
    const context='__context',bindings=ctx?'const '+ctx.getText(sf)+'=__context;':'';
    const returns=setup.body.statements.filter(ts.isReturnStatement);let render=returns.at(-1)?.expression;
    while(render&&ts.isBinaryExpression(render)&&render.operatorToken.kind===ts.SyntaxKind.CommaToken)render=render.right;
    if(!render||!ts.isArrowFunction(render))throw Error('原组件渲染结构变化：'+component);
    const locals=names(setup,sf).filter(n=>n!=='f'||component!=='AiConfigForm');
    let prefix=locals.length?'const {'+locals.join(',')+'}=__job('+JSON.stringify(component)+','+props+','+context+');':'';
    if(component==='AiConfigForm'){
     let provider;for(const s of setup.body.statements)if(ts.isVariableStatement(s))for(const d of s.declarationList.declarations)if(d.name.getText(sf)==='f')provider=d.initializer.getText(sf);
     if(!provider)throw Error('缺少原服务商列表');prefix+='const f='+provider+';';
    }
    let body=custom.has(component)?'return __render('+JSON.stringify(component)+','+props+','+context+');':prefix+'return '+render.getText(sf)+';';
    if(component==='PieChart')body=setup.body.getText(sf).slice(1,-1).replace('return ','__chart(()=>i);return ').replace('trigger:"item"','trigger:"item",renderMode:"richText"').replace('i=NB(n.value)','i=NB(n.value,null,{renderer:"svg"})');
    if(component==='AiConfigForm'){
     const form=render.getText(sf).replace('a(It).openUrl(s.value.url,{external:!0})','__open(s.value.url)').replace('placeholder:""','placeholder:"留空保留已存密钥",type:"password","show-password":!0');
     body='const __open=__job("links",'+props+',__context);'+prefix+'return __wrap('+form+',__job("settingsExtras",'+props+',__context));';
    }
    if(component==='KnowledgeView')body='const __text=__job("label",'+props+',__context);'+body.replace(/y\[5\]\|\|\(y\[5\]=\[fs\(" 生成 "\)\]\)/,'[fs(__text())]').replace('"native-type":"submit"','"native-type":"button"').replace('maxlength:"30"','maxlength:"200"').replace('maxlength:"500"','maxlength:"2000"');
    if(component==='JobAnalysisView')body=body.replace('确定要清空所有聊天记录？','确定要清空所有已采集职位？');
    edit(setup,'setup('+props+',__context){'+bindings+body+'}');usesUi=true;changed.push(name+':'+component);return;
   }
  }
  // Vite原站的绝对图片路径改为本地资源；原字节保留在original目录。
  if(ts.isStringLiteral(node)&&/^\/ai\/job\/assets\//.test(node.text))edit(node,'new URL('+JSON.stringify('./'+node.text.split('/').at(-1))+',import.meta.url).href');
  ts.forEachChild(node,walk);
 }
 walk(sf);
 for(const s of sf.statements){
  if(ts.isExpressionStatement(s)){
   const t=s.getText(sf);
   if(name==='index-Bfkv8pFi.js'&&(t.includes('relList')&&t.includes('modulepreload')||t.startsWith('Sn(Ym,"aiConfig"')||t.startsWith('zf.')))edit(s,'');
   if(name==='JobView-DmmzGjQr.js'&&t==='Le.react=new is;')edit(s,'');
  }
  if(ts.isVariableStatement(s))for(const d of s.declarationList.declarations){const key=d.name.getText(sf);
   if(name==='index-Bfkv8pFi.js'&&['eae','rae','zf'].includes(key))edit(d.initializer,'undefined');
   if(name==='index-Bfkv8pFi.js'&&key==='tl')edit(d.initializer,'factory=>factory()');
   if(name==='JobRepository-CvsTQyXb.js'&&key==='H')edit(d.initializer,'undefined');
   if(name==='KnowledgeView-Cjfd7KFs.js'&&['NI','It'].includes(key))edit(d.initializer,'undefined');
   if(name==='Window-li_yC5Al.js'&&key==='ft')edit(d.initializer,'Object.freeze({})');
  }
 }
 edits.sort((a,b)=>b[0]-a[0]);let previous=Infinity,text=raw;
 for(const[start,end,replacement]of edits){if(end>previous)throw Error('派生补丁重叠：'+name);text=text.slice(0,start)+replacement+text.slice(end);previous=start;}
 if(usesUi)text='import{setup as __job,render as __render,chart as __chart,wrap as __wrap}from"../setup.js";\n'+text;
 if(name==='KnowledgeView-Cjfd7KFs.js')text+='\nexport{Ve as originalMarkdown};\n';
 if(name==='index-Bfkv8pFi.js')text+='\nexport{$k as createApp,K as ref,Yt as reactive,I as computed,Fe as inject,se as defineComponent,he as node,rt as onMounted,zn as onBeforeUnmount,xe as watch,Ve as nextTick,yQ as elementPlus};\n';
 writeFileSync(out+name,'// 固定部署派生；来源和补丁见SOURCE_FILES.json与scripts/adapt-widgetjs-job.mjs。\n'+text);
}
writeFileSync(root+'/DERIVED_FILES.json',JSON.stringify({revision:provenance.revision,components:changed,removed:['原站入口和路由启动','OpenAI默认客户端','原Dexie实例','Electron浏览器桥','组件原setup副作用'],preserved:'原组件compiled render、props、emits、作用域标记、CSS、图标和原部署字节'},null,2)+'\n');
writeFileSync(root+'/UPSTREAM_NOTICES.txt',notices.join('\n\n')+'\n');
console.log('已派生固定AI直聘资源，替换'+changed.length+'处组件宿主逻辑');
