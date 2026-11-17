import coins from '../coin-common/coins.json' with {type:'json'};
import {signOf} from '../metal/metal-model.js';
import {goldChange} from '../gold-source/gold-model.js';
import {validateProxy} from '../coin-common/network-model.js';
export {coins};
export const fallbacks={BTC:'bitcoin.png',ETH:'ethereum.png',XRP:'xrp.png',BNB:'bnb.png',SOL:'solana.png',TRX:'tron.png',DOGE:'dogecoin.png',SUI:'sui.png',OKB:'okb.png',TRUMP:'trump.png'};
export function validateSettings(value) {
  validateProxy(value);
  if(![3000,10000,30000,60000].includes(value.refreshMs))throw new Error('刷新间隔无效');
  if(!['background','foreground','primary','border'].every(key=>/^#[0-9a-f]{6}$/i.test(value[key]))||!Number.isInteger(value.radius)||value.radius<0||value.radius>40)throw new Error('颜色或圆角设置无效');
  return value;
}
export function readSnapshot(value) {
  if(!value||value.provider!=='OKX'||value.source!=='coin-currencies'||value.channel!=='cup-tickers-3s'||typeof value.connectionId!=='string'||!value.connectionId||!Number.isSafeInteger(value.epoch)||value.epoch<0||!['connecting','connected','retrying'].includes(value.connection)||!Array.isArray(value.quotes)||value.quotes.length>coins.length)throw new Error('网页币种报价来源或连接身份无效');
  const quotes=new Map();
  for(const quote of value.quotes) {
    if(!coins.some(coin=>coin.ccy===quote.currency)||quotes.has(quote.currency)||signOf(quote.price)<=0||!Number.isSafeInteger(quote.timestamp)||quote.timestamp<=0||!Number.isFinite(new Date(quote.timestamp).getTime())||!Number.isSafeInteger(quote.receivedAt)||quote.receivedAt<=0||!Number.isSafeInteger(quote.epoch)||quote.epoch<1||quote.epoch>value.epoch)throw new Error('币种报价身份、价格或时间无效');
    if(quote.reference!=null&&signOf(quote.reference)<=0)throw new Error('24小时参考价格无效');
    quotes.set(quote.currency,{...quote,change:goldChange({latestPrice:quote.price,yesterdayClose:quote.reference})});
  }
  return {...value,quotes,currencyErrors:value.currencyErrors||{}};
}
export function quoteIsOld(snapshot,quote,now=Date.now(),interval=3000) {return !quote||snapshot.connection!=='connected'||quote.epoch!==snapshot.epoch||Boolean(snapshot.currencyErrors[quote.currency])||quote.timestamp>now+5000||now-quote.timestamp>Math.max(15000,interval*2);}
export function createDisplay() {
  const quotes=new Map();let connectionId=null,paintedAt=0;
  return {quotes,reset(){connectionId=null;paintedAt=0;quotes.clear();},
    update(snapshot,interval,now=Date.now()) {
      if(!snapshot)return quotes;
      if(connectionId!==snapshot.connectionId){quotes.clear();connectionId=snapshot.connectionId;paintedAt=0;}
      const due=!paintedAt||now-paintedAt>=interval;
      for(const [ccy,quote]of snapshot.quotes)if(due||!quotes.has(ccy)||quotes.get(ccy).epoch!==snapshot.epoch)quotes.set(ccy,quote);
      if(due)paintedAt=now;
      return quotes;
    },
  };
}
export function createFeed(load,changed=()=>{}) {
  const state={snapshot:null,error:'',loading:false};let revision=0,disposed=false;
  return {state,
    async refresh(){if(disposed)return;const request=++revision;state.loading=true;changed(state);try{const response=await load();if(disposed||request!==revision)return;state.snapshot=readSnapshot(response.data);state.error='';}catch(error){if(!disposed&&request===revision)state.error=String(error?.message||error);}finally{if(!disposed&&request===revision){state.loading=false;changed(state);}}},
    reset(){revision++;state.snapshot=null;state.error='';state.loading=false;changed(state);},
    dispose(){disposed=true;revision++;},
  };
}
