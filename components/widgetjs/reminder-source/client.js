// 固定提醒业务的设置与播放状态；不接收自定义音频地址，不调度后台任务。
export function defaults(source){
  if(source==='sit')return {enabled:false,sitInterval:30,breakInterval:5,mouseCheckInterval:5};
  if(source==='phone')return {enabled:false,rules:['squidward','tea','pill','study'].map((preset,i)=>({preset,enabled:false,minuteOfDay:[1080,900,720,720][i],workdays:[1,2,3,4,5]}))};
  return {enabled:false};
}
const clone=value=>JSON.parse(JSON.stringify(value));
export function createSettings(host,source,changed=()=>{}){
  let pending,serial=0;
  const state={document:{revision:0,config:defaults(source),runtime:{}},draft:defaults(source),dirty:false,saving:false,loading:false,error:'',uncertain:false};
  const emit=()=>changed({...state});
  async function read(reload){
    if(host.preview)return;
    if(reload){state.loading=true;emit();}
    const current=serial;
    try{const value=await host.request(source+'.get');if(current!==serial||value.revision<state.document.revision)return;
      state.document=value;
      if(reload||!state.dirty){state.draft=clone(value.config);state.dirty=false;state.error='';}
      emit();
    }catch(error){if(current===serial){state.error=error.message;emit();}}finally{if(reload){state.loading=false;emit();}}
  }
  return {state,load:()=>read(true),refresh:()=>read(false),edit(value){
    if(state.uncertain||state.saving)throw Error('上次保存结果未确定，请先重试');
    state.draft=clone(value);state.dirty=true;state.error='';emit();
  },async save(){
    if(host.preview||state.saving||state.loading)return;
    serial++;state.saving=true;state.error='';
    pending??={requestId:crypto.randomUUID(),expectedRevision:state.document.revision,config:clone(state.draft)};emit();
    try{const value=await host.request(source+'.configure',pending);state.document=value;state.draft=clone(value.config);state.dirty=false;state.uncertain=false;pending=undefined;
    }catch(error){state.error=error.message;state.uncertain=/超时|timeout|连接.*断|结果未确定/i.test(error.message);if(!state.uncertain)pending=undefined;
    }finally{state.saving=false;emit();}
  }};
}
export function parseLyrics(text){return text.split(/\r?\n/).flatMap(line=>{const match=/^\[(\d+):(\d+(?:\.\d+)?)\](.*)$/.exec(line);return match?[{at:Number(match[1])*60+Number(match[2]),text:match[3]}]:[];}).sort((a,b)=>a.at-b.at);}
export function lyricAt(lines,seconds,fallback){let result=fallback;for(const line of lines){if(line.at>seconds)break;result=line.text;}return result;}
export function clockText(ms){const n=Math.max(0,Math.floor(ms/10));return [Math.floor(n/6000),Math.floor(n/100)%60,n%100].map(n=>String(n).padStart(2,'0')).join(':');}
export function progress(remaining,total){return total>0?Math.min(100,Math.max(0,remaining/total*100)):0;}
export function createPlayback(elements,report,changed=()=>{}){
  let sequence=0,closed=false;
  const state={playing:false,hanging:false,error:''};
  const emit=()=>changed({...state});
  const pause=()=>{elements.voice?.pause();elements.hangup?.pause();};
  async function notify(value){try{await report(value);}catch(error){if(!closed){state.error='播放状态未能保存：'+error.message;emit();}}}
  async function fail(error){state.playing=false;state.hanging=false;state.error='音频未能播放，请点击接听重试';emit();await notify(error?.name==='NotAllowedError'?'blocked':'failed');}
  return {state,async start(){
    if(closed)return;const current=++sequence;pause();state.error='';state.hanging=false;state.playing=false;emit();
    try{if(!elements.voice)throw Error('音频未就绪');elements.voice.currentTime=0;await elements.voice.play();if(closed||current!==sequence){pause();return;}state.playing=true;emit();await notify('playing');}
    catch(error){if(!closed&&current===sequence)await fail(error);}
  },async ended(){
    if(closed)return;const current=++sequence;state.playing=false;state.hanging=true;emit();
    try{if(!elements.hangup)throw Error('挂断音频未就绪');await elements.hangup.play();if(closed||current!==sequence){pause();return;}}
    catch(error){if(!closed&&current===sequence)await fail(error);}
  },async finished(){if(!closed){state.playing=false;state.hanging=false;emit();await notify('ended');}},async failed(error){if(!closed){++sequence;pause();await fail(error);}},stop(){closed=true;++sequence;pause();state.playing=false;state.hanging=false;emit();}};
}
