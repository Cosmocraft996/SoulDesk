const object=value=>value&&typeof value==='object'&&!Array.isArray(value);
const ipv4=ip=>typeof ip==='string'&&ip.split('.').length===4&&ip.split('.').every(part=>/^\d{1,3}$/.test(part)&&Number(part)<=255);
function connection(api,optional=false){
  if(!object(api)||typeof api.url!=='string'||typeof api.token!=='string')throw new Error('服务地址或令牌格式无效');
  if(optional&&!api.url&&!api.token)return {url:'',token:''};
  let url;try{url=new URL(api.url);}catch{throw new Error('监控服务地址无效');}
  if(!['http:','https:'].includes(url.protocol)||url.username||url.password||url.search||url.hash||api.url.length>2048||/\s/.test(api.url)||!api.token||api.token.length>4096||/[\x00-\x1f\x7f]/.test(api.token))throw new Error('监控服务地址或令牌无效');
  return {url:url.href.replace(/\/+$/,''),token:api.token};
}
export function validateServer(input){
  if(!object(input)||typeof input.id!=='string'||!input.id||input.id.length>128||typeof input.name!=='string'||!input.name.trim()||input.name.length>30||!ipv4(input.ip))throw new Error('服务器名称、IPv4或标识格式无效');
  const api={...input.api,interval:input.api?.interval??10};connection(api,true);if(!Number.isInteger(api.interval)||api.interval<1||api.interval>3600)throw new Error('刷新间隔须为1～3600秒');
  for(const key of ['platform','region','intranetIp','ipV6','note','account'])if(input[key]!==undefined&&(typeof input[key]!=='string'||input[key].length>16384))throw new Error('服务器说明字段格式无效');
  return {...structuredClone(input),api};
}
export function restoreConfig(stored){
  if(!object(stored))throw new Error('服务器配置不是有效对象，请保留原数据');if(!Object.keys(stored).length)return {version:1,servers:[],master:{url:'',token:''}};
  if(stored.version!==1||!Array.isArray(stored.servers)||stored.servers.length>128)throw new Error('服务器配置版本或列表格式无效');
  const servers=stored.servers.map(validateServer),ids=new Set();for(const server of servers){if(ids.has(server.id))throw new Error('服务器标识重复');ids.add(server.id);}
  return {...structuredClone(stored),servers,master:connection(stored.master??{url:'',token:''},true)};
}
export function createServerStore(stored,persist,changed=(_state)=>{},uuid=()=>crypto.randomUUID()){
  let value=restoreConfig(stored),queue=Promise.resolve(),disposed=false,revision=0;const state={busy:false,error:''};const notice=()=>{if(!disposed)changed(state);};
  function mutate(change){if(disposed)return Promise.reject(new Error('组件视图已关闭'));const task=queue.then(async()=>{
    if(disposed)throw new Error('组件视图已关闭');state.busy=true;state.error='';notice();
    try{const next=structuredClone(value),result=change(next),valid=restoreConfig(next);if(new TextEncoder().encode(JSON.stringify(valid)).length>4*1024*1024)throw new Error('服务器配置超过4MiB上限');await persist(valid);value=valid;revision++;return result;}
    catch(error){state.error=String(error?.message||error);throw error;}finally{state.busy=false;notice();}
  });queue=task.catch(()=>{});return task;}
  return {state,get revision(){return revision;},snapshot:()=>structuredClone(value),dispose(){disposed=true;},
    save(input){return mutate(next=>{const server=validateServer({...input,id:input.id||uuid()});const index=next.servers.findIndex(item=>item.id===server.id);if(index<0)next.servers.push(server);else next.servers[index]=server;return server.id;});},
    remove(id){return mutate(next=>{if(!next.servers.some(server=>server.id===id))throw new Error('服务器已不存在');next.servers=next.servers.filter(server=>server.id!==id);});},
    setMaster(master){return mutate(next=>{next.master=connection(master,true);});},
    merge(incoming,expectedRevision){return mutate(next=>{if(expectedRevision!==revision)throw new Error('本地配置已改变，请重新同步');if(!Array.isArray(incoming)||incoming.length>128)throw new Error('同步服务器列表格式无效');for(const input of incoming){const server=validateServer({...input,id:input.id||uuid(),api:input.api??{url:'',token:'',interval:10}});const index=next.servers.findIndex(item=>item.id===server.id);if(index<0)next.servers.push(server);else next.servers[index]=server;}});}
  };
}
export function monitorArgs(kind,server,master={url:'',token:''}){
  if(!['stats','info'].includes(kind))throw new Error('未开放的监控操作');const target=connection(server.api),proxy=connection(master,true);
  return proxy.url?{kind,connection:proxy,forward:target}:{kind,connection:target};
}
export function createServerReader(fetch,changed=()=>{},now=()=>Date.now()){
  const values=new Map();let disposed=false,inFlight=0;
  function get(server,master={url:'',token:''}){const key=JSON.stringify([server.api,master]),previous=values.get(server.id);if(previous?.key===key)return previous;const state={key,data:null,receivedAt:null,busy:false,error:'',nextAt:0};values.set(server.id,state);return state;}
  return {get,retain(ids){const keep=new Set(ids);for(const id of values.keys())if(!keep.has(id))values.delete(id);},dispose(){disposed=true;},async refresh(server,master,force=false){
    const state=get(server,master);if(disposed||state.busy||inFlight>=4||!force&&state.nextAt>now())return false;state.busy=true;inFlight++;changed();
    try{const result=await fetch(monitorArgs('stats',server,master));if(disposed||values.get(server.id)!==state)return false;if(!object(result?.data)||!Number.isSafeInteger(result.receivedAt)||result.receivedAt<0)throw new Error('监控指标响应无效');state.data=result.data;state.receivedAt=result.receivedAt;state.error='';return true;}
    catch(error){if(!disposed&&values.get(server.id)===state)state.error=String(error?.message||error);return false;}
    finally{state.busy=false;inFlight--;state.nextAt=now()+(server.api?.interval??10)*1000;if(!disposed&&values.get(server.id)===state)changed();}
  }};
}
const percent=value=>typeof value==='number'&&Number.isFinite(value)&&value>=0&&value<=100?value:null;
const bytes=value=>Number.isSafeInteger(value)&&value>=0?value:null;
export function prettyBytes(value){if(bytes(value)===null)return '—';const units=['B','KB','MB','GB','TB'];let index=0;while(value>=1024&&index<units.length-1){value/=1024;index++;}return Math.round(value)+' '+units[index];}
export function maskIp(ip){return ipv4(ip)?ip.split('.')[0]+'.**.**.**':ip;}
export function metrics(reading,interval=10,now=Date.now()){
  const data=reading?.data??{},mem=data.mem??{},disk=data.fsSize?.[0]??{};const total=bytes(mem.total),available=bytes(mem.available),active=bytes(mem.active),used=bytes(disk.used),size=bytes(disk.size);
  const badTime=!Number.isSafeInteger(reading?.receivedAt)||reading.receivedAt>now+5000,stale=!badTime&&now-reading.receivedAt>Math.max(15000,interval*3000);
  return {cpu:badTime?null:percent(data.currentLoad?.currentLoad),memory:badTime||total===null||total===0||available===null||available>total?null:(total-available)/total*100,disk:badTime?null:percent(disk.use),memoryText:badTime?'— / —':(active!==null&&total!==null&&active<=total?prettyBytes(active):'—')+' / '+prettyBytes(total),diskText:badTime?'— / —':(used!==null&&size!==null&&used<=size?prettyBytes(used):'—')+' / '+prettyBytes(size),stale,status:!reading?.data?(reading?.error||'尚未读取指标'):badTime?'接收时间无效，指标暂不展示':reading.error?'旧数据 · '+reading.error:stale?'旧数据 · 已超过预期刷新时间':'接收于 '+new Date(reading.receivedAt).toLocaleTimeString('zh-CN',{hour12:false})+' · 服务未提供采样时间'};
}
