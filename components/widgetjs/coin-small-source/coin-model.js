import coins from '../coin-common/coins.json' with {type:'json'};
import {signOf} from '../metal/metal-model.js';
import {goldChange} from '../gold-source/gold-model.js';
import {validateProxy} from '../coin-common/network-model.js';
export {coins};
export function validateSettings(value){
  if(!value||!coins.some(coin=>coin.type===value.coin)||![30000,60000,180000,300000].includes(value.refreshMs))throw new Error('币种或刷新间隔无效');
  return validateProxy(value);
}
export function readIndex(value,instrument){
  if(!coins.some(coin=>coin.type===instrument)||!value||value.instrument!==instrument||signOf(value.price)<=0||!Number.isSafeInteger(value.timestamp)||value.timestamp<=0||!Number.isFinite(new Date(value.timestamp).getTime()))throw new Error('OKX指数身份、价格或时间无效');
  if(value.reference!=null&&(signOf(value.reference)<=0||!['open24h','sodUtc8'].includes(value.basis)))throw new Error('指数参考价格或基准无效');
  return {...value,change:goldChange({latestPrice:value.price,yesterdayClose:value.reference})};
}
export function isOld(value,now=Date.now(),interval=60000){return !value||now-value.timestamp>Math.max(15000,interval*2)||value.timestamp>now+5000;}
export function createCoin(load,changed=()=>{}){
  const state={instrument:null,quote:null,points:[],error:'',loading:false};let revision=0,disposed=false;
  async function refresh(){
    if(disposed||!state.instrument)return;const request=++revision,instrument=state.instrument;state.loading=true;state.error='';changed(state);
    try{const response=await load(instrument);if(disposed||request!==revision)return;const value=readIndex(response.data,instrument);if(state.quote&&value.timestamp<state.quote.timestamp)throw new Error('指数返回较旧时间，保留上次报价');
      state.quote=value;const last=state.points.at(-1);if(last?.timestamp===value.timestamp)state.points[state.points.length-1]={timestamp:value.timestamp,price:value.price};else state.points=[...state.points.slice(-47),{timestamp:value.timestamp,price:value.price}];
    }catch(error){if(!disposed&&request===revision)state.error=String(error?.message||error);}
    finally{if(!disposed&&request===revision){state.loading=false;changed(state);}}
  }
  return {state,refresh,
    restore(response,instrument){
      if(disposed||!response)return false;
      const value=readIndex(response.data,instrument),samples=response.samples;
      if(!Array.isArray(samples)||!samples.length||samples.length>48)throw new Error('临时指数采样数量无效');
      const points=samples.map((sample,index)=>{
        readIndex({...value,price:sample.price,timestamp:sample.timestamp},instrument);
        if(sample.timestamp>value.timestamp||(index&&sample.timestamp<=samples[index-1].timestamp))throw new Error('临时指数采样时间无效');
        return {timestamp:sample.timestamp,price:sample.price};
      });
      if(points.at(-1).timestamp!==value.timestamp||points.at(-1).price!==value.price)throw new Error('临时指数末点与报价不符');
      state.instrument=instrument;state.quote=value;state.points=points;state.error='';changed(state);return true;
    },
    select(instrument){if(!coins.some(coin=>coin.type===instrument))throw new Error('币种不在原目录');if(state.instrument!==instrument){state.quote=null;state.points=[];}state.instrument=instrument;return refresh();},
    reset(){revision++;state.instrument=null;state.quote=null;state.points=[];state.error='';state.loading=false;changed(state);},
    dispose(){disposed=true;revision++;},
  };
}
