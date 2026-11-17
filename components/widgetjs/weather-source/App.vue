<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, provide, reactive, ref, watchEffect } from 'vue';
import Small from './views/Small.vue';
import Medium from './views/Medium.vue';
import Large from './views/Large.vue';
import { weatherContext } from './context';
import { createWeatherModel, observationIsOld, periodIsPast } from './weather-model.js';
const props = defineProps<{ host:any; variant:string; initialSettings:Record<string,any> }>();
const settings = ref({ ...props.initialSettings }), dialog = ref<HTMLDialogElement>();
const state = reactive({ config:{ configured:false,enabled:false,apiHost:'' },now:null,daily:null,indices:[],errors:{},loading:false });
const model = createWeatherModel((method:string,args:any) => props.host.request(method,args), { preview:props.host.preview,large:props.variant==='weather-large',getSettings:()=>settings.value }, state);
const draft = ref<Record<string,any>>({}), apiHost = ref(''), apiKey = ref(''), enabled = ref(false), connectionStatus = ref(''), saveError = ref(''), searchText = ref(''), searchStatus = ref(''), cities = ref<any[]>([]), saving = ref(false);
const views:Record<string,any> = { 'weather-small':Small,'weather-medium':Medium,'weather-large':Large };
let timer:ReturnType<typeof setInterval>, ageTimer:ReturnType<typeof setInterval>, searchEpoch=0, disposed=false;
const clock = ref(Date.now());
const old = computed(() => state.now && (observationIsOld(state.now,clock.value) || !!(state.errors as any).now));
const errors = computed(() => Object.entries(state.errors).map(([key,value]) => ({now:'实况',daily:'三日预报',indices:'生活指数'}[key] || key)+'：'+value).join('；'));
const style = computed(() => ({ '--widget-font-size':settings.value.fontSize+'px','--widget-border-radius':settings.value.radius+'px',color:settings.value.foreground }));
function openSettings() { draft.value={...settings.value};apiHost.value=state.config.apiHost;apiKey.value='';enabled.value=state.config.enabled;connectionStatus.value='';saveError.value='';cities.value=[];searchStatus.value='';dialog.value?.showModal(); }
provide(weatherContext, { host:props.host,model,settings,openSettings,clock });
watchEffect(() => { if(typeof document!=='undefined') { document.documentElement.style.setProperty('--widget-font-size',settings.value.fontSize+'px');document.documentElement.style.fontSize=settings.value.fontSize+'px'; } });
async function saveConnection() {
  if(props.host.preview)return;saving.value=true;connectionStatus.value='';
  try { const next=await props.host.request('weather.configure',{apiHost:apiHost.value.trim(),apiKey:apiKey.value,enabled:enabled.value});model.reset();state.config=next;apiKey.value='';connectionStatus.value='连接设置已保存';await model.refresh(); }
  catch(error:any){connectionStatus.value='连接设置未保存：'+error.message;}finally{saving.value=false;}
}
async function clearConnection() { if(props.host.preview)return;saving.value=true;try{state.config=await props.host.request('weather.clear');model.reset();apiKey.value='';apiHost.value='';enabled.value=false;connectionStatus.value='本实例天气凭据已清除';}catch(error:any){connectionStatus.value='清除失败：'+error.message;}finally{saving.value=false;} }
async function search() { const request=++searchEpoch;searchStatus.value='正在搜索…';try{const result=await model.lookup(searchText.value);if(disposed||request!==searchEpoch)return;cities.value=result;searchStatus.value=result.length?'选择城市后保存城市与外观':'没有结果；请先保存并启用连接';}catch(error:any){if(!disposed&&request===searchEpoch)searchStatus.value='搜索失败：'+error.message;} }
async function saveSettings() {
  if(props.host.preview)return;saving.value=true;saveError.value='';
  try {const next={...draft.value,radius:Number(draft.value.radius),fontSize:Number(draft.value.fontSize)};if(!next.cityName?.trim()||!/^[A-Za-z0-9]{2,32}$/.test(next.cityId)||!['C','F'].includes(next.unit))throw Error('城市或单位无效');new Intl.DateTimeFormat('zh-CN',{timeZone:next.timezone}).format();const saved=await props.host.request('settings.update',next);const changed=saved.cityId!==settings.value.cityId;settings.value=saved;if(changed)model.reset();dialog.value?.close();await model.refresh();}
  catch(error:any){saveError.value='城市与外观未保存：'+error.message;}finally{saving.value=false;}
}
async function start() {
  if(disposed)return;
  try {if(!props.host.preview)await model.readStatus();}catch(error:any){if(!disposed)(state.errors as any).now=error.message;}
  if(disposed)return;
  try {await props.host.ready();if(disposed)return;void model.refresh();timer=setInterval(()=>{if(!state.loading)void model.refresh();},3600000);ageTimer=setInterval(()=>{clock.value=Date.now();},60000);}
  catch(error:any){if(!disposed)(state.errors as any).now=error.message;}
}
function dispose(){disposed=true;searchEpoch++;clearInterval(timer);clearInterval(ageTimer);model.dispose();}
onMounted(start);
onBeforeUnmount(dispose);
</script>
<template>
  <main id="weather-root" :style="style"><component :is="views[variant]" />
    <div class="weather-actions"><button type="button" :disabled="host.preview || state.loading" aria-label="刷新天气" @click="model.refresh">↻</button><button type="button" aria-label="天气设置" @click="openSettings">⚙</button></div>
    <footer v-if="state.now" class="weather-source"><span :class="{old}">{{ old?'旧数据 · ':'' }}观测 {{ (state.now as any).obsTime }}</span><span>和风天气 · {{ settings.unit==='F'?'华氏':'摄氏' }} · 一小时刷新</span></footer>
    <p v-if="state.daily && state.daily.some((day:any)=>periodIsPast(day.fxDate,settings.timezone,clock))" class="weather-period-old">三日预报含过期日期，保留原日期</p><p v-if="state.indices.length && state.indices.some((day:any)=>periodIsPast(day?.date,settings.timezone,clock))" class="weather-index-old">生活指数日期已过期或未提供</p><p v-if="errors && state.now" class="weather-error" role="status">{{ errors }}；保留已有数据</p>
    <dialog ref="dialog" class="weather-settings"><h2>天气设置</h2><form @submit.prevent="saveConnection"><h3>本实例连接</h3><label>API Host<input v-model="apiHost" placeholder="你的专属域名.qweatherapi.com" autocomplete="off"></label><label>API Key<input v-model="apiKey" type="password" autocomplete="new-password" :placeholder="state.config.configured?'留空保留已保存密钥':'填写自己的密钥'"></label><label><input v-model="enabled" type="checkbox">启用联网查询（使用自有服务额度）</label><p>凭据在本机加密保存。更换Host需重新输入密钥。</p><p role="status">{{ connectionStatus }}</p><button :disabled="saving || host.preview">保存连接</button><button type="button" :disabled="saving || host.preview" @click="clearConnection">清除本实例凭据</button></form>
      <form @submit.prevent="saveSettings"><h3>城市与外观</h3><label>搜索城市<input v-model="searchText" @keydown.enter.prevent="search"></label><button type="button" :disabled="saving || host.preview" @click="search">搜索</button><p>{{ searchStatus }}</p><button v-for="city in cities" :key="city.id" type="button" @click="draft.cityId=city.id;draft.cityName=city.name;draft.timezone=city.tz">{{ city.country }} {{ city.adm1 }} {{ city.adm2 }} {{ city.name }}</button><p>待保存城市：{{ draft.cityName }} · {{ draft.cityId }}</p><label>温度单位<select v-model="draft.unit"><option value="C">摄氏°C</option><option value="F">华氏°F</option></select></label><label>字号<input v-model="draft.fontSize" type="number" min="10" max="30" required></label><label>圆角<input v-model="draft.radius" type="number" min="0" max="40" required></label><label>文字颜色<input v-model="draft.foreground" type="color"></label><p role="status">{{ saveError }}</p><button :disabled="saving || host.preview">保存城市与外观</button><button type="button" @click="dialog?.close()">关闭</button></form>
    </dialog>
  </main>
</template>
