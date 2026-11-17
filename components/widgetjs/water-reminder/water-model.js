const valid=value=>value&&/^\d{4}-\d{2}-\d{2}$/.test(value.day)&&Number.isSafeInteger(value.sampledAt)&&value.sampledAt>=0&&Number.isInteger(value.count)&&value.count>=0&&value.count<=4294967295&&Number.isInteger(value.targetCup)&&value.targetCup>=1&&value.targetCup<=20&&typeof value.enableReminder==='boolean'&&Number.isInteger(value.interval)&&value.interval>=5&&value.interval<=100;
export function createWater(request,changed=()=>{},requestId=()=>crypto.randomUUID()){
  const state={snapshot:null,error:null,pending:null,busy:false};let disposed=false;
  async function send(method,args){
    if(disposed||state.busy)return false;state.busy=true;changed();
    try{
      const value=await request(method,args);if(disposed)return false;if(!valid(value))throw new Error('喝水数据格式无效');
      state.snapshot=value;if(method==='water.change')state.pending=null;if(!state.pending)state.error=null;return true;
    }catch(error){if(!disposed)state.error=String(error?.message||error);return false;}
    finally{state.busy=false;if(!disposed)changed();}
  }
  return {state,refresh:()=>send('water.get'),change(operation){
    if(disposed||state.busy||state.pending||!['add','reset'].includes(operation))return Promise.resolve(false);
    state.pending={requestId:requestId(),operation};return send('water.change',state.pending);
  },retry(){return state.pending?send('water.change',state.pending):send('water.get');},dispose(){disposed=true;}};
}
export function waveProgress(count,target){return Number.isInteger(count)&&count>=0&&Number.isInteger(target)&&target>0?Math.min(100,count/target*100):null;}
