// 只读取固定部署的语法树，不执行上游脚本或引入其运行时。
import ts from 'typescript';
import {readFileSync,writeFileSync,readdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
const root='components/widgetjs/deadline-source';
export function extractDeadlineSvg(source){
  const tree=ts.createSourceFile('deadline.js',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
  let svg;
  function visit(node){if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&node.expression.text==='e'&&ts.isStringLiteral(node.arguments[0])&&node.arguments[0].text==='svg'){if(svg)throw new Error('原稿SVG不唯一');svg=node;}ts.forEachChild(node,visit);}visit(tree);
  if(!svg)throw new Error('未找到原稿SVG');
  const escape=value=>String(value).replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;').replaceAll('>','&gt;');
  function literal(node){if(ts.isStringLiteral(node)||ts.isNoSubstitutionTemplateLiteral(node))return node.text;if(ts.isNumericLiteral(node))return Number(node.text);if(ts.isPrefixUnaryExpression(node)&&node.operator===ts.SyntaxKind.MinusToken&&ts.isNumericLiteral(node.operand))return-Number(node.operand.text);throw new Error('原稿出现非静态属性');}
  function element(node){
    if(!ts.isCallExpression(node)||!ts.isIdentifier(node.expression)||node.expression.text!=='e')throw new Error('原稿出现可执行表达式');
    const tag=literal(node.arguments[0]);if(!['svg','g','defs','rect','clipPath','use','path','circle'].includes(tag))throw new Error('原稿出现未知标签：'+tag);
    let attributes='';const attrs=node.arguments[1];
    if(attrs&&attrs.kind!==ts.SyntaxKind.NullKeyword){if(!ts.isObjectLiteralExpression(attrs))throw new Error('非静态属性对象');for(const item of attrs.properties){if(!ts.isPropertyAssignment(item))throw new Error('非静态属性');const key=ts.isIdentifier(item.name)?item.name.text:literal(item.name),value=literal(item.initializer);if(!/^[a-z][a-z0-9:_-]*$/i.test(key)||/^on/i.test(key)||key==='style'||(/href$/i.test(key)&&!String(value).startsWith('#')))throw new Error('不接受外部引用或脚本属性');attributes+=' '+key+'="'+escape(value)+'"';}}
    let children='';const child=node.arguments[2];if(child){if(!ts.isArrayLiteralExpression(child))throw new Error('非静态子节点');children=child.elements.map(element).join('');}
    return '<'+tag+attributes+'>'+children+'</'+tag+'>';
  }
  return element(svg)+'\n';
}
export function portDeadline(){
  const source=readFileSync(root+'/original/DeadlineWidgetView-IuZHNBhI.js','utf8');
  writeFileSync(root+'/artwork.svg',extractDeadlineSvg(source));
  writeFileSync(root+'/original-style.css',readFileSync(root+'/original/DeadlineWidgetView-C9hVe0nS.css','utf8').replaceAll('[data-v-afe3cbb0]',''));
  const files=readdirSync(root+'/original').sort().map(name=>({path:'original/'+name,url:name==='main-excerpts.txt'?'https://itime.fun/web/assets/index-BJrdQu13.js':'https://itime.fun/web/assets/'+name,sha256:createHash('sha256').update(readFileSync(root+'/original/'+name)).digest('hex'),kind:name==='main-excerpts.txt'?'verbatim-excerpts':'deployed-asset'}));
  writeFileSync(root+'/SOURCE_FILES.json',JSON.stringify({entry:'https://itime.fun/web/',deployment:'index-BJrdQu13.js',entrySha256:'79ba25621b91084bb06fa4a3e78888eee28f03767949ac7049ab19b7dee5cf06',upstreamId:'itime.fun.deadline',author:'LYP（目录登记）',license:'公开部署未附许可声明；按用户明确的自用范围保留来源，不推定为GPL',files},null,2)+'\n');
  console.log('Deadline原SVG与动画样式已提取；未执行上游脚本');
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)portDeadline();
