import { readMetric } from '../hardware-common/metrics.js';
export { createMetrics } from '../hardware-common/metrics.js';
const titles={cpu:'CPU',memory:'Mem',gpu:'GPU'};
export function gauge(snapshot,type,now=Date.now()){
  const title=titles[type]||'未知指标',empty=status=>({label:title+' —',angle:-80,status,valid:false});
  const metric=readMetric(snapshot,type,now);
  if(metric.percent===null)return empty(metric.status);
  return {label:title+' '+Math.round(metric.percent)+'%',angle:-80+metric.percent*.8,status:'',valid:true};
}
