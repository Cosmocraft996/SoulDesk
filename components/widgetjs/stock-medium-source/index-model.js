import { decimal, signOf } from '../metal/metal-model.js';
export const indices=[{code:'000001',exchange:'SH',name:'上证指数'},{code:'399001',exchange:'SZ',name:'深证成指'},{code:'399006',exchange:'SZ',name:'创业板指'}];
export function readIndex(value,code){
  const expected=indices.find(item=>item.code===code);
  if(!value||!expected||value.code!==expected.code||value.exchange!==expected.exchange||value.market!=='ab'||value.type!=='index'||typeof value.name!=='string'||!value.name||signOf(value.price)<=0)throw new Error('指数身份或价格无效');
  decimal(value.ratio);if(typeof value.stockStatusInfo!=='string'||typeof value.sourceTimeText!=='string')throw new Error('指数来源状态无效');
  return {...value};
}
export function createIndices(load,changed=()=>{}){
  const state={rows:indices.map(item=>({...item,quote:null,error:'',loading:false,fetchedAt:0})),loading:false};let revision=0,disposed=false;
  return {state,
    async refresh(){
      if(disposed)return;const request=++revision;state.loading=true;state.rows.forEach(row=>{row.loading=true;row.error='';});changed(state);
      await Promise.all(state.rows.map(async row=>{
        try{const response=await load(row.code);if(disposed||request!==revision)return;
          const quote=readIndex(response.data,row.code);if(!Number.isSafeInteger(response.fetchedAt)||response.fetchedAt<=0)throw new Error('读取时刻无效');
          row.quote=quote;row.fetchedAt=response.fetchedAt;
        }catch(error){if(!disposed&&request===revision)row.error=String(error?.message||error);}
        finally{if(!disposed&&request===revision){row.loading=false;changed(state);}}
      }));
      if(!disposed&&request===revision){state.loading=false;changed(state);}
    },dispose(){disposed=true;revision++;},
  };
}
