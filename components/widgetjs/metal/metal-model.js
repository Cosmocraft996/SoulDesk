const codes=['XAUUSD','XAGUSD','XCUUSD','XPTUSD'];
export function decimal(value) {
  if(typeof value!=='string'||value.length>96)throw new Error('报价必须为十进制字符串');
  const match=/^(-?)(\d{1,64})(?:\.(\d{1,28}))?$/.exec(value);
  if(!match)throw new Error('报价格式无效');
  return {negative:match[1]==='-',digits:BigInt(match[2]+(match[3]||'')),scale:(match[3]||'').length};
}
export function signOf(value){const part=decimal(value);return part.digits===0n?0:part.negative?-1:1;}
export function formatDecimal(value,places=2){
  const part=decimal(value),target=10n**BigInt(places),denominator=10n**BigInt(part.scale);
  const rounded=(part.digits*target*2n+denominator)/(denominator*2n),text=rounded.toString().padStart(places+1,'0');
  const whole=(places?text.slice(0,-places):text).replace(/\B(?=(\d{3})+(?!\d))/g,',');
  return (part.negative&&rounded!==0n?'-':'')+whole+(places?'.'+text.slice(-places):'');
}
export function readMetals(data){
  if(!Array.isArray(data)||data.length!==codes.length)throw new Error('贵金属报价不完整');
  return data.map((quote,index)=>{
    if(quote.code!==codes[index]||typeof quote.name!=='string'||signOf(quote.price)<=0||typeof quote.stale!=='boolean'||typeof quote.marketState!=='string'||typeof quote.updatedAt!=='string'||!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)$/i.test(quote.updatedAt)||!Number.isFinite(Date.parse(quote.updatedAt)))throw new Error('贵金属报价身份、价格或时间无效');
    if(quote.quoteAgeSeconds!=null&&(!Number.isSafeInteger(quote.quoteAgeSeconds)||quote.quoteAgeSeconds<0))throw new Error('源报价时效无效');
    decimal(quote.changePercent);return {...quote};
  });
}
export function quoteIsOld(quote,now=Date.now()){const time=Date.parse(quote.updatedAt);return quote.stale||(quote.quoteAgeSeconds!=null&&quote.quoteAgeSeconds>90)||!Number.isFinite(time)||now-time>90000||time>now+5000;}
export function createMetals(load,changed=()=>{}){
  const state={quotes:[],error:'',loading:false,fetchedAt:0};let revision=0,disposed=false;
  return {state,
    async refresh(){
      if(disposed)return;const request=++revision;state.loading=true;state.error='';changed(state);
      try{
        const response=await load();if(disposed||request!==revision)return;
        const quotes=readMetals(response.data);
        if(state.quotes.some((previous,index)=>Date.parse(quotes[index].updatedAt)<Date.parse(previous.updatedAt)))throw new Error('内容源返回较旧报价，保留上次数据');
        state.quotes=quotes;state.fetchedAt=response.fetchedAt;
      }catch(error){if(!disposed&&request===revision)state.error=String(error?.message||error);}
      finally{if(!disposed&&request===revision){state.loading=false;changed(state);}}
    },
    dispose(){disposed=true;revision++;},
  };
}
