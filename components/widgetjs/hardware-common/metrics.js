const types=['cpu','memory','gpu'];
export function snapshotStatus(snapshot,now=Date.now()){
  if(!snapshot)return '等待本机读数';
  if(!Number.isSafeInteger(snapshot.sampledAt)||now-snapshot.sampledAt>10000||snapshot.sampledAt-now>5000)return '读数已过期或时钟发生变化';
  return '';
}
export function readMetric(snapshot,type,now=Date.now()){
  if(!types.includes(type))return {percent:null,status:'监控类型无效'};
  const status=snapshotStatus(snapshot,now);if(status)return {percent:null,status};
  const metric=snapshot[type];
  if(!metric||!Number.isFinite(metric.percent)||metric.percent<0||metric.percent>100)return {percent:null,status:metric?.error||'暂无有效读数'};
  return {percent:metric.percent,status:''};
}
function validSnapshot(value){
  if(!value||!Number.isSafeInteger(value.sampledAt)||value.sampledAt<0)throw new Error('采样时间无效');
  for(const key of types){
    const metric=value[key];
    if(!metric||!(metric.percent===null||(Number.isFinite(metric.percent)&&metric.percent>=0&&metric.percent<=100)))throw new Error('占用率无效');
    if(metric.percent===null&&!(typeof metric.error==='string'&&metric.error))throw new Error('无效读数缺少原因');
  }
  return value;
}
export function createMetrics(fetch,changed){
  const state={snapshot:null,error:null,loading:false};let disposed=false;
  return {state,async refresh(){
    if(disposed||state.loading)return;
    state.loading=true;changed();
    try{const value=validSnapshot(await fetch());if(disposed)return;
      if(state.snapshot&&value.sampledAt<state.snapshot.sampledAt)throw new Error('采样时间倒退');
      state.snapshot=value;state.error=null;
    }catch(error){if(!disposed)state.error=String(error?.message||error);}
    finally{state.loading=false;if(!disposed)changed();}
  },dispose(){disposed=true;}};
}
