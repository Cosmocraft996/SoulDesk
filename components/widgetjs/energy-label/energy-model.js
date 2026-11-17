import { readMetric } from '../hardware-common/metrics.js';
export const colors=['#1b814a','#93bd30','#f8d701','#f8a002','#e30f1b'];
export function energy(snapshot,now=Date.now()){
  const cpu=readMetric(snapshot,'cpu',now),memory=readMetric(snapshot,'memory',now);
  const cpuText=cpu.percent===null?'—':Math.round(cpu.percent)+'%',memoryText=memory.percent===null?'—':Math.round(memory.percent)+'%';
  if(cpu.percent===null||memory.percent===null)return {level:null,score:null,color:'#707882',top:0,cpuText,memoryText,status:[...new Set([cpu.status,memory.status].filter(Boolean))].join('；')};
  const score=cpu.percent*.8+memory.percent*.2,level=Math.min(5,Math.max(1,Math.ceil(score/20)));
  return {level,score,color:colors[level-1],top:36*(level-1),cpuText,memoryText,status:''};
}
