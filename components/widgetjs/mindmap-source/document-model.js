const defaults=()=>({theme:'system',showGrid:true,showCheckboxes:true,sidebarOpen:true});
const object=value=>value&&typeof value==='object'&&!Array.isArray(value);
function text(value,label,max=4096){if(typeof value!=='string'||value.length>max)throw new Error(label+'格式无效');return value;}
function validateTree(root){
  const ids=new Set(),seen=new Set(),stack=[[root,0]];let count=0;
  while(stack.length){const [node,depth]=stack.pop();if(depth>128)throw new Error('导图层级超过128层');if(!object(node)||seen.has(node))throw new Error('导图树无效或存在循环');seen.add(node);
    text(node.id,'节点标识',256);text(node.name,'节点文字');if(!node.id||ids.has(node.id))throw new Error('节点标识为空或重复');ids.add(node.id);if(++count>10000)throw new Error('导图节点超过10000项');
    for(const key of ['checked','collapsed'])if(node[key]!==undefined&&typeof node[key]!=='boolean')throw new Error('节点状态格式无效');
    for(const key of ['url','type'])if(node[key]!==undefined)text(node[key],'节点字段',2048);
    if(node.children!==undefined){if(!Array.isArray(node.children))throw new Error('导图子节点格式无效');for(const child of node.children)stack.push([child,depth+1]);}
  }
}
function validateDocument(doc){if(!object(doc)||!text(doc.id,'导图标识',256))throw new Error('导图标识无效');text(doc.topic,'主题',256);text(doc.emoji,'图标',64);text(doc.group,'分组',256);if(!doc.topic.trim())throw new Error('导图主题不能为空');for(const key of ['createdAt','updatedAt'])if(!Number.isSafeInteger(doc[key])||doc[key]<0)throw new Error('导图日期无效');validateTree(doc.mindmap);}
export function restoreDocuments(stored){
  if(!object(stored))throw new Error('思维导图存储格式无效，请保留原数据');if(!Object.keys(stored).length)return {version:1,documents:[],preferences:defaults()};
  if(stored.version!==1||!Array.isArray(stored.documents)||stored.documents.length>512)throw new Error('思维导图存储版本或文档格式无效');
  const ids=new Set();for(const doc of stored.documents){validateDocument(doc);if(ids.has(doc.id))throw new Error('导图标识重复');ids.add(doc.id);}
  const preferences={...defaults(),...stored.preferences};if(!['system','light','dark'].includes(preferences.theme)||['showGrid','showCheckboxes','sidebarOpen'].some(key=>typeof preferences[key]!=='boolean'))throw new Error('思维导图偏好格式无效');
  return structuredClone({...stored,preferences});
}
export function readImport(raw){
  let value;try{value=JSON.parse(raw);}catch{throw new Error('不是有效的JSON导图文件');}
  const mindmap=object(value)&&object(value.mindmap)?value.mindmap:value;validateTree(mindmap);
  const topic=object(value)&&typeof value.topic==='string'?value.topic:mindmap.name;
  const doc={topic,emoji:typeof value.emoji==='string'?value.emoji:'📝',group:typeof value.group==='string'?value.group:'默认分组',mindmap};
  validateDocument({...doc,id:'import',createdAt:0,updatedAt:0});return structuredClone(doc);
}
export function createDocumentStore(stored,persist,changed=(_state)=>{},limit=32*1024*1024,id=()=>crypto.randomUUID(),now=()=>Date.now()){
  let value=restoreDocuments(stored),signature=JSON.stringify(value),revision=0,saved=0,blocked='',running=null,disposed=false;
  const state={status:'saved',error:''};const notice=()=>{if(!disposed)changed(state);};
  const size=string=>new TextEncoder().encode(string).length;
  if(size(signature)>limit)throw new Error('已有导图超出实例存储上限，请先导出原数据');
  async function drain(){while(saved!==revision&&!blocked){const writing=revision,snapshot=structuredClone(value);state.status='saving';state.error='';notice();try{await persist(snapshot);}catch(error){state.status='error';state.error=String(error?.message||error);notice();return false;}saved=writing;}state.status=blocked?'error':'saved';state.error=blocked;notice();return !blocked;}
  function flush(){if(blocked)return Promise.resolve(false);if(running)return running;running=drain().finally(()=>{running=null;});return running;}
  function mutate(change){if(disposed)throw new Error('组件视图已关闭');const next=structuredClone(value),result=change(next);const valid=restoreDocuments(next),nextSignature=JSON.stringify(valid);if(nextSignature===signature)return result;value=valid;signature=nextSignature;revision++;blocked=size(signature)>limit?'导图超过实例存储上限，最新草稿未保存，请缩小或导出文件':'';if(blocked){state.status='error';state.error=blocked;notice();}else void flush();notice();return result;}
  function find(data,key){const doc=data.documents.find(item=>item.id===key);if(!doc)throw new Error('导图已不存在，请重新选择');return doc;}
  function add(data,doc){const at=now(),key=id();data.documents.push({...structuredClone(doc),id:key,createdAt:at,updatedAt:at});return key;}
  return {state,snapshot:()=>structuredClone(value),flush,retry:flush,dispose(){disposed=true;},
    create(topic,emoji='📝',group='默认分组'){return mutate(data=>add(data,{topic:topic.trim(),emoji,group:group.trim()||'默认分组',mindmap:{id:id(),name:topic.trim(),children:[]}}));},
    updateTree(key,mindmap){validateTree(mindmap);return mutate(data=>{const doc=find(data,key);doc.mindmap=structuredClone(mindmap);doc.updatedAt=now();});},
    rename(key,topic,emoji,group){return mutate(data=>{const doc=find(data,key);doc.topic=topic.trim();doc.emoji=emoji;doc.group=group.trim()||'默认分组';doc.mindmap.name=doc.topic;doc.updatedAt=now();});},
    remove(key){return mutate(data=>{find(data,key);data.documents=data.documents.filter(doc=>doc.id!==key);});},
    setPreferences(patch){return mutate(data=>{Object.assign(data.preferences,patch);});},
    importDocument(doc){return mutate(data=>add(data,doc));}
  };
}
