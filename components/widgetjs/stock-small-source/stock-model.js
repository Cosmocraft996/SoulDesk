import {decimal,signOf} from '../metal/metal-model.js';
import {goldChange} from '../gold-source/gold-model.js';
export const markets={0:'SZ/BJ',1:'SH',116:'HK',105:'NASDAQ',106:'US-106',107:'US-107',113:'NYSE',114:'US-114',155:'US-155',156:'US-156',157:'US-157'};
export function validateSelection(value){
  if(!value||!Object.hasOwn(markets,value.marketId)||typeof value.code!=='string'||!/^[a-z0-9.^-]{1,32}$/i.test(value.code)||value.code.includes('..')||!/[a-z0-9]/i.test(value.code)||typeof value.name!=='string'||!value.name||value.name.length>128)throw new Error('证券代码、市场或名称无效');
  return {code:value.code,marketId:value.marketId,name:value.name};
}
export function readStock(value,selection){
  validateSelection(selection);
  if(!value||value.code!==selection.code||value.marketId!==selection.marketId||typeof value.name!=='string'||!value.name||!Number.isInteger(value.decimal)||value.decimal<0||value.decimal>12||!Array.isArray(value.times)||!Array.isArray(value.values)||value.times.length!==value.values.length||value.values.length>3000)throw new Error('证券分时身份或格式无效');
  value.values.forEach(price=>{if(signOf(price)<=0)throw new Error('证券分时价格无效');});
  value.times.forEach((time,index)=>{if(typeof time!=='string'||!/^\d{4}-\d\d-\d\d \d\d:\d\d(?::\d\d)?$/.test(time)||index>0&&time<value.times[index-1])throw new Error('证券分时时间无效或倒序');});
  if(value.previousClose!=null&&signOf(value.previousClose)<=0)throw new Error('证券昨收无效');
  const price=value.values.at(-1)||null,change=price?goldChange({latestPrice:price,yesterdayClose:value.previousClose}):null;
  return {...value,times:[...value.times],values:[...value.values],price,change};
}
export function createStock(load,changed=()=>{}){
  const state={selection:null,quote:null,error:'',loading:false,fetchedAt:0};let revision=0,disposed=false;
  async function refresh(){
    if(disposed||!state.selection)return;const request=++revision,selection={...state.selection};state.loading=true;state.error='';changed(state);
    try{const result=await load(selection);if(disposed||request!==revision)return;const value=readStock(result.data,selection);if(state.quote?.times.length&&value.times.length&&value.times.at(-1)<state.quote.times.at(-1))throw new Error('返回较旧分时，保留上次数据');state.quote=value;state.fetchedAt=Number.isSafeInteger(result.fetchedAt)?result.fetchedAt:0;}
    catch(error){if(!disposed&&request===revision)state.error=String(error?.message||error);}
    finally{if(!disposed&&request===revision){state.loading=false;changed(state);}}
  }
  return {state,refresh,
    select(value){const selected=validateSelection(value);if(state.selection?.code!==selected.code||state.selection?.marketId!==selected.marketId){state.quote=null;state.fetchedAt=0;}state.selection=selected;return refresh();},
    dispose(){disposed=true;revision++;},
  };
}
