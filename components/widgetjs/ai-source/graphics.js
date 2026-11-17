// 固定Markmap/Mermaid图形引擎；模型输出仅为内容，不执行配置或外部资源。
import createDOMPurify from'dompurify';
function purifier(){return createDOMPurify(window);}
function cssSafe(text){if(/\\|@import|expression\s*\(|javascript:|-moz-binding/i.test(text)||/url\s*\(/i.test(text.replace(/url\(\s*["']?#[\w:.-]+["']?\s*\)/gi,'')))throw Error('图形包含外部或不支持的样式资源');}
export function sanitizeSvg(text){
  if(typeof text!=='string'||text.length>4*1024*1024)throw Error('图形超过4MiB');
  const clean=purifier().sanitize(text,{USE_PROFILES:{svg:true,svgFilters:true,html:true},ADD_TAGS:['foreignObject'],HTML_INTEGRATION_POINTS:{foreignobject:true},FORBID_TAGS:['script','iframe','object','embed','a','image','img','audio','video','link','use','form','input','button'],FORBID_ATTR:['href','xlink:href','src','srcset','action','formaction'],RETURN_DOM_FRAGMENT:true});
  const svg=clean.querySelector('svg');if(!svg)throw Error('没有有效的图形SVG');if(svg.hasAttribute('style'))cssSafe(svg.getAttribute('style'));for(const style of svg.querySelectorAll('style'))cssSafe(style.textContent);for(const node of svg.querySelectorAll('[style]'))cssSafe(node.getAttribute('style'));
  return new XMLSerializer().serializeToString(svg);
}
export async function mindmapData(markdown){
  if(typeof markdown!=='string'||markdown.length>524288)throw Error('导图文本超过容量');
  const{Transformer,builtInPlugins}=await import('markmap-lib');
  const transformer=new Transformer(builtInPlugins.filter(plugin=>plugin.name==='frontmatter'));transformer.md.set({html:false,linkify:false});
  transformer.md.renderer.rules.image=(tokens,index)=>transformer.md.utils.escapeHtml(tokens[index].content??'');transformer.md.renderer.rules.link_open=()=>'<span>';transformer.md.renderer.rules.link_close=()=>'</span>';
  const{root}=transformer.transform(markdown);const purify=purifier();let count=0;const visit=(node,depth)=>{if(++count>3000||depth>64)throw Error('导图超过3000节点或64层');node.content=purify.sanitize(node.content,{ALLOWED_TAGS:['p','br','strong','em','code','pre','span','del','mark','sup','sub','ins','table','tr','td','th','ul','ol','li'],ALLOWED_ATTR:[]});if(node.payload){const{fold,lines}=node.payload;node.payload={...(Number.isInteger(fold)?{fold}:{}),...(Array.isArray(lines)?{lines}: {})};}for(const child of node.children??[])visit(child,depth+1);};visit(root,0);return root;
}
export async function mountMindmap(svg,root){const{Markmap}=await import('markmap-view');const map=Markmap.create(svg,{embedGlobalCSS:false,duration:0,autoFit:false,maxWidth:360,zoom:true,pan:true});await map.setData(root);return map;}
let chartSequence=0,renderQueue=Promise.resolve();
export async function mermaidSvg(code,theme='light',isCurrent=()=>true){
  if(typeof code!=='string'||!code.trim()||code.length>100000)throw Error('图表源码须为1到100000字');
  const normalized=code.replace(/^```(?:mermaid)?\s*\n([\s\S]*?)\n```\s*$/i,'$1');
  if(/%%\s*\{|^\s*---/.test(normalized))throw Error('此组件不执行模型输出的图形配置，请仅填写图表源码');
  const current=++chartSequence;
  const render=async()=>{if(!isCurrent())throw Error('旧图形渲染已取消');const{default:mermaid}=await import('mermaid');if(!isCurrent())throw Error('旧图形渲染已取消');mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:theme==='dark'?'dark':'default',maxTextSize:100000,flowchart:{htmlLabels:false},suppressErrorRendering:true});const{svg}=await mermaid.render('souldesk_ai_chart_'+current,normalized);return sanitizeSvg(svg);};
  const result=renderQueue.catch(()=>{}).then(render);renderQueue=result.catch(()=>{});return result;
}
export async function savePng(host,svg,name,styleSource=svg){
  if(host.preview)return false;const exported=svg.cloneNode(true);
  const computed=getComputedStyle(styleSource);if(computed.color)exported.style.color=computed.color;if(computed.fontFamily)exported.style.fontFamily=computed.fontFamily;for(const property of ['--markmap-text-color','--markmap-code-color','--markmap-code-bg','--markmap-circle-open-bg']){const value=computed.getPropertyValue(property);if(value)exported.style.setProperty(property,value);}
  if(svg.classList.contains('markmap')){const{globalCSS}=await import('markmap-view');const style=document.createElementNS('http://www.w3.org/2000/svg','style');style.textContent=globalCSS;exported.prepend(style);}
  const clean=sanitizeSvg(new XMLSerializer().serializeToString(exported)),image=new Image();
  const box=svg.viewBox?.baseVal;const width=Math.ceil(box?.width||svg.getBoundingClientRect().width),height=Math.ceil(box?.height||svg.getBoundingClientRect().height);
  if(!Number.isFinite(width)||!Number.isFinite(height)||width<1||height<1||width>8192||height>8192||width*height>16000000)throw Error('导出尺寸无效或超过1600万像素');
  const loaded=new Promise((resolve,reject)=>{image.onload=resolve;image.onerror=()=>reject(Error('SVG图像转换失败'));});let binary='';const bytes=new TextEncoder().encode(clean);for(let offset=0;offset<bytes.length;offset+=8192)binary+=String.fromCharCode(...bytes.subarray(offset,offset+8192));image.src='data:image/svg+xml;base64,'+btoa(binary);await loaded;
  const scale=Math.min(2,Math.sqrt(16000000/(width*height))),canvas=document.createElement('canvas');canvas.width=Math.floor(width*scale);canvas.height=Math.floor(height*scale);const context=canvas.getContext('2d');if(!context)throw Error('无法创建导出画布');context.scale(scale,scale);context.drawImage(image,0,0,width,height);const data=canvas.toDataURL('image/png').split(',')[1];const result=await host.request('documents.save',{fileName:name,extensions:['png'],data});return result;
}
