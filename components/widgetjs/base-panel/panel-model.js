import { readMetric,snapshotStatus } from '../hardware-common/metrics.js';
const units=['B','KB','MB','GB','TB'],gib=1024n**3n;
function counter(value){return typeof value==='string'&&/^\d{1,20}$/.test(value)&&BigInt(value)<=18446744073709551615n?BigInt(value):null;}
export function prettyBytes(value,digits=0){
  if(![0,2].includes(digits))return '—';
  if(typeof value==='number'){
    if(!Number.isFinite(value)||value<0||value>Number.MAX_SAFE_INTEGER)return '—';let unit=0;
    while(value>=1024&&unit<units.length-1){value/=1024;unit++;}
    return (digits===0?String(Math.round(value)):value.toFixed(digits))+' '+units[unit];
  }
  const bytes=counter(value);if(bytes===null)return '—';let unit=0,scale=1n;
  while(bytes>=scale*1024n&&unit<units.length-1){scale*=1024n;unit++;}
  const factor=10n**BigInt(digits),rounded=(bytes*factor+scale/2n)/scale;
  return (digits===0?String(rounded):String(rounded/factor)+'.'+String(rounded%factor).padStart(digits,'0'))+' '+units[unit];
}
export function panel(snapshot,now=Date.now()){
  const status=snapshotStatus(snapshot,now),details=status?null:snapshot?.details,errors=[];
  const result={};
  for(const key of ['cpu','memory','gpu']){
    const value=readMetric(snapshot,key,now);if(value.status)errors.push(value.status);
    result[key]={percent:value.percent,label:value.percent===null?'—':Math.round(value.percent)+'%',subtitle:'—',detail:value.status};
  }
  if(!details&&!status)errors.push('请升级主程序以读取硬件细节');
  result.cpu.subtitle=Number.isSafeInteger(details?.cpuLogicalCount)&&details.cpuLogicalCount>0?details.cpuLogicalCount+' C':'— C';
  result.cpu.detail||='本机CPU总占用率；C为逻辑处理器数';
  const memory=counter(details?.memoryTotalBytes);result.memory.subtitle=memory!==null&&memory>0n?String((memory+gib-1n)/gib)+' GB':'— GB';
  result.memory.detail||='本机物理内存；总量按GiB向上取整，保留原GB标签';
  const temp=details?.gpuTemperature,hasTemp=result.gpu.percent!==null&&typeof details?.gpuAdapter==='string'&&Number.isFinite(temp?.celsius)&&temp.celsius>0&&temp.celsius<=200;
  result.gpu.subtitle=hasTemp?Math.round(temp.celsius)+' ℃':'— ℃';
  result.gpu.detail=result.gpu.detail||('最繁忙GPU引擎；主温度对应适配器 '+(details?.gpuAdapter||'尚未提供')+(hasTemp?'':' · '+(temp?.error||'温度不可用')));
  if(details&&!hasTemp&&temp?.error)errors.push(temp.error);
  const network=details?.network;result.network={download:prettyBytes(network?.downloadBytesPerSecond),upload:prettyBytes(network?.uploadBytesPerSecond),received:prettyBytes(network?.receivedBytes,2),sent:prettyBytes(network?.sentBytes,2),detail:'所有已连接的非回环接口合计；虚拟网卡可能重复计数。累计值从系统接口计数器读取。'};
  if(network?.error)errors.push(network.error);result.status=[...new Set(errors)].join('；');
  return result;
}
