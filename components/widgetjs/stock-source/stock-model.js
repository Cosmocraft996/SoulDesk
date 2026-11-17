import {decimal,signOf} from '../metal/metal-model.js';
import {goldChange} from '../gold-source/gold-model.js';
import {validateSelection,markets} from '../stock-small-source/stock-model.js';
export {markets};
export const stockKey=stock=>`${stock.marketId}:${stock.code}`;
function holding(value){
  if(value==null||value==='')return '';
  if(typeof value!=='string'||!/^\d{1,20}(?:\.\d{1,12})?$/.test(value))throw Error('手工持仓须为非负十进制文本，最多20位整数及12位小数');
  return value;
}
export function normalizePortfolio(value){
  if(value==null||Object.keys(value).length===0)return{version:1,stocks:[]};
  if(value.version!==1||!Array.isArray(value.stocks)||value.stocks.length>50)throw Error('自选列表格式或版本无效，最多50项');
  const seen=new Set();const stocks=value.stocks.map(stock=>{const result={...validateSelection(stock),holdingPrice:holding(stock.holdingPrice),holdingShares:holding(stock.holdingShares)};const key=stockKey(result);if(seen.has(key))throw Error('同一市场证券不能重复');seen.add(key);return result;});
  return{version:1,stocks};
}
function scaled(value,scale){const d=decimal(value);return(d.negative?-d.digits:d.digits)*10n**BigInt(scale-d.scale);}
function plain(value,scale){const negative=value<0n,text=(negative?-value:value).toString().padStart(scale+1,'0');return(negative?'-':'')+(scale?(text.slice(0,-scale)+'.'+text.slice(-scale)).replace(/\.?0+$/,''):text);}
export function holdingMetrics(stock,quote){
  const result={change:null,returnRate:null,returnAmount:null};if(!quote)return result;
  if(signOf(quote.price)<=0)throw Error('报价无效');
  result.change=goldChange({latestPrice:quote.price,yesterdayClose:quote.previousClose});
  const cost=holding(stock.holdingPrice),shares=holding(stock.holdingShares);if(!cost||signOf(cost)<=0)return result;
  result.returnRate=goldChange({latestPrice:quote.price,yesterdayClose:cost}).percent;
  if(shares==='')return result;
  const scale=Math.max(decimal(quote.price).scale,decimal(cost).scale),quantity=decimal(shares);
  result.returnAmount=plain((scaled(quote.price,scale)-scaled(cost,scale))*quantity.digits,scale+quantity.scale);
  return result;
}
export function readQuote(value,selection){
  if(!value||value.code!==selection.code||value.marketId!==selection.marketId||typeof value.name!=='string'||!value.name||value.name.length>128||!Number.isInteger(value.decimal)||value.decimal<0||value.decimal>12||signOf(value.price)<=0||value.identityBasis!=='request-secid')throw Error('证券报价身份或数值无效');
  if(value.previousClose!=null&&signOf(value.previousClose)<=0)throw Error('昨收无效');
  if(decimal(value.price).scale>value.decimal||value.previousClose!=null&&decimal(value.previousClose).scale>value.decimal)throw Error('报价小数位超出来源声明精度');
  if(value.sourceTime!=null&&(!Number.isSafeInteger(value.sourceTime)||value.sourceTime<=0||value.sourceTime>253402300799))throw Error('来源时间无效');
  return{...value};
}
export function createPortfolio(load,write,initial,options={}){
  const state={portfolio:normalizePortfolio(initial),quotes:{},loading:false,saving:false};let disposed=false,revision=0;
  return{state,
    async save(value){
      if(disposed||options.preview)throw Error('当前视图不能保存');if(state.saving)throw Error('正在保存，请稍候');
      const next=normalizePortfolio(value);state.saving=true;
      try{const result=normalizePortfolio(await write(next));if(disposed)return;revision++;state.portfolio=result;state.loading=false;const keys=new Set(result.stocks.map(stockKey));state.quotes=Object.fromEntries(Object.entries(state.quotes).filter(([key])=>keys.has(key)));return result;}
      finally{state.saving=false;}
    },
    async refresh(){
      if(disposed||options.preview)return;const request=++revision,stocks=state.portfolio.stocks.map(stock=>({...stock}));state.loading=true;
      const results=await Promise.allSettled(stocks.map(load));if(disposed||request!==revision)return;
      for(let i=0;i<stocks.length;i++){
        const key=stockKey(stocks[i]),previous=state.quotes[key]||{data:null,fetchedAt:0,error:''};
        try{if(results[i].status==='rejected')throw results[i].reason;const response=results[i].value,data=readQuote(response.data,stocks[i]);
          if(previous.data?.sourceTime&&(!data.sourceTime||data.sourceTime<previous.data.sourceTime))throw Error('返回较旧或缺少来源时间的报价，保留上次数据');
          state.quotes[key]={data,error:'',fetchedAt:Number.isSafeInteger(response.fetchedAt)?response.fetchedAt:0};
        }catch(error){state.quotes[key]={...previous,error:String(error?.message||error)};}
      }
      state.loading=false;
    },
    dispose(){disposed=true;revision++;},
  };
}
