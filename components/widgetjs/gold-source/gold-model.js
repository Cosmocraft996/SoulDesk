import { decimal, signOf } from '../metal/metal-model.js';
function scaled(value,scale){const part=decimal(value);return (part.negative?-part.digits:part.digits)*10n**BigInt(scale-part.scale);}
function plain(value,scale){const negative=value<0n,text=(negative?-value:value).toString().padStart(scale+1,'0');return (negative?'-':'')+(scale?(text.slice(0,-scale)+'.'+text.slice(-scale)).replace(/\.?0+$/,''):text);}
export function readGold(value){
  if(!value||value.instrument!=='Au99.99'||value.code!=='AU9999'||signOf(value.latestPrice)<=0||!Number.isSafeInteger(value.timestamp)||value.timestamp<=0||!Number.isFinite(new Date(value.timestamp).getTime()))throw new Error('黄金合约、价格或时间无效');
  for(const key of ['yesterdayClose','min','max'])if(value[key]!=null&&signOf(value[key])<=0)throw new Error('黄金统计价格无效');
  return {...value};
}
export function goldChange(quote){
  if(quote.yesterdayClose==null)return null;
  const scale=Math.max(decimal(quote.latestPrice).scale,decimal(quote.yesterdayClose).scale),current=scaled(quote.latestPrice,scale),previous=scaled(quote.yesterdayClose,scale);
  if(previous<=0n)return null;const difference=current-previous,absolute=difference<0n?-difference:difference;
  const rounded=(absolute*20000n+previous)/(2n*previous),text=rounded.toString().padStart(3,'0');
  return {amount:plain(difference,scale),percent:(difference<0n&&rounded!==0n?'-':'')+text.slice(0,-2)+'.'+text.slice(-2),sign:difference<0n?-1:difference>0n?1:0};
}
function trendTime(value){
  if(typeof value!=='string'||!/^\d{4}-\d\d-\d\d \d\d:\d\d(?::\d\d)?$/.test(value))throw new Error('黄金分时时间无效');
  const time=Date.parse(value.replace(' ','T')+(value.length===16?':00':'')+'+08:00');
  if(!Number.isFinite(time))throw new Error('黄金分时时间无效');return time;
}
export function readTrends(value){
  if(!value||value.code!=='AU9999'||!Array.isArray(value.values)||!Array.isArray(value.times)||value.values.length!==value.times.length||value.values.length>3000)throw new Error('黄金分时数据或身份无效');
  const times=value.times.map(trendTime);value.values.forEach(price=>{if(signOf(price)<=0)throw new Error('黄金分时价格无效');});
  if(times.some((time,index)=>index>0&&time<times[index-1]))throw new Error('黄金分时时间倒序');
  return {code:value.code,times:[...value.times],values:[...value.values],timestamps:times};
}
function chinaDay(time){return new Date(time+8*3600000).toISOString().slice(0,10);}
export function chartMatchesQuote(series,quote){return Boolean(quote&&series?.timestamps.length&&chinaDay(series.timestamps.at(-1))===chinaDay(quote.timestamp));}
export function quoteIsOld(quote,now=Date.now()){return !quote||now-quote.timestamp>30000||quote.timestamp>now+5000;}
export function chartGeometry(values,current,width,height){
  if(!values.length)return {points:[],latestY:null};
  if(!Number.isFinite(width)||!Number.isFinite(height)||width<=0||height<=8)throw new Error('分时画布尺寸无效');
  const scale=Math.max(...values.map(value=>decimal(value).scale),decimal(current).scale);
  const numbers=values.map(value=>scaled(value,scale)),last=scaled(current,scale);
  let minimum=numbers[0],maximum=numbers[0];for(const value of [...numbers,last]){if(value<minimum)minimum=value;if(value>maximum)maximum=value;}
  const range=maximum-minimum||1n,plotHeight=height-8;
  const y=value=>4+plotHeight-Number((value-minimum)*1000000n/range)/1000000*plotHeight;
  return {points:numbers.map((value,index)=>({x:numbers.length===1?width/2:index/(numbers.length-1)*width,y:y(value)})),latestY:y(last)};
}
export function createGold(loadQuote,loadChart,changed=()=>{}){
  const state={quote:null,series:null,error:'',chartError:'',loading:false};let revision=0,disposed=false;
  const announce=()=>{if(!disposed)changed(state);};
  return {state,
    async refresh(){
      if(disposed)return;const request=++revision;state.loading=true;state.error='';state.chartError='';announce();
      const current=()=>!disposed&&request===revision;
      await Promise.all([
        Promise.resolve().then(loadQuote).then(response=>{if(!current())return;const value=readGold(response.data);if(state.quote&&value.timestamp<state.quote.timestamp)throw new Error('报价源返回较旧数据，保留上次报价');state.quote=value;announce();}).catch(error=>{if(current()){state.error=String(error?.message||error);announce();}}),
        Promise.resolve().then(loadChart).then(response=>{if(!current())return;const value=readTrends(response.data);if(state.series?.timestamps.length&&value.timestamps.length&&value.timestamps.at(-1)<state.series.timestamps.at(-1))throw new Error('分时源返回较旧数据，保留上次分时');state.series=value;announce();}).catch(error=>{if(current()){state.chartError=String(error?.message||error);announce();}}),
      ]);
      if(current()){state.loading=false;announce();}
    },dispose(){disposed=true;revision++;},
  };
}
