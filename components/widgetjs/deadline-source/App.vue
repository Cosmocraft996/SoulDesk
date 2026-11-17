<script setup>
import{computed,nextTick,onMounted,onBeforeUnmount,ref}from'vue';
import{useTransition,TransitionPresets}from'@vueuse/core';
import{deadline,localDay,settingsFromDraft,validateSettings}from'./deadline-model.js';
import artwork from'./artwork.svg?raw';
const props=defineProps({host:Object,initialSettings:Object});
const settings=ref(validateSettings(props.initialSettings)),now=ref(Date.now()),dialog=ref(),draft=ref({}),saveError=ref(''),saving=ref(false);
const progress=computed(()=>deadline(settings.value,now.value));
const fill=ref(-100),death=ref(0);
const animatedFill=useTransition(fill,{duration:3000,transition:TransitionPresets.easeOutCubic});
const animatedDeath=useTransition(death,{duration:3000,transition:TransitionPresets.easeOutCubic});
const theme=computed(()=>({'--a0831b64':animatedFill.value+'%','--v6602f3ee':animatedDeath.value+'px','--widget-primary-color':settings.value.primaryColor,'--widget-background-color':settings.value.background,'--widget-border-radius':settings.value.radius+'px'}));
let timer,alive=true;
function update(){now.value=Date.now();fill.value=progress.value.fillX;death.value=progress.value.deathX;}
onMounted(async()=>{await nextTick();update();await props.host.ready();if(alive)timer=setInterval(update,1000);if(!props.host.preview&&new URLSearchParams(location.search).get('verify')==='widgetjs')await verifySession();});
onBeforeUnmount(()=>{alive=false;clearInterval(timer);});
function open(){const value=settings.value;draft.value={title:value.title,startDate:localDay(value.startTime),endDate:localDay(value.endTime),primaryColor:value.primaryColor,background:value.background,radius:value.radius};saveError.value='';dialog.value.showModal();}
async function save(){
  if(props.host.preview||saving.value)return;saving.value=true;saveError.value='';
  try{const next=settingsFromDraft(settings.value,draft.value);for(const key of ['primaryColor','background'])if(!CSS.supports('color',next[key]))throw new Error('颜色格式无效');const result=validateSettings(await props.host.request('settings.update',next));if(!alive)return;settings.value=result;update();dialog.value.close();}
  catch(error){if(alive)saveError.value='设置未保存：'+String(error?.message||error);}
  finally{saving.value=false;}
}
// 仅显式隔离入口执行真实WebView表单；不把程序化动作称为人工视觉验收。
async function verifySession(){
  const old=await props.host.request('storage.get'),previous=old.verification;
  let invalidSaveRejected=previous?.invalidSaveRejected===true;
  const label='期限-'+props.host.instanceId.slice(0,8),background='#'+props.host.instanceId.replaceAll('-','').slice(0,6);
  if(!previous?.uiVerified){
    open();await nextTick();const before=JSON.stringify(await props.host.request('settings.get'));
    draft.value.radius=51;await save();invalidSaveRejected=JSON.stringify(await props.host.request('settings.get'))===before&&saveError.value.includes('未保存');
    draft.value.radius=12;draft.value.title=label;draft.value.background=background;await nextTick();
    document.getElementById('deadline-form').requestSubmit();
    for(let n=0;n<100&&saving.value;n++)await new Promise(resolve=>setTimeout(resolve,50));
    if(saving.value||dialog.value.open)throw new Error('Deadline表单保存未完成：'+saveError.value);
  }
  await nextTick();const saved=await props.host.request('settings.get');
  const stylesLoaded=getComputedStyle(document.querySelector('.widget')).borderRadius==='12px'&&getComputedStyle(document.getElementById('red-flame')).animationDuration.split(',')[0].trim()==='20s';
  const imagesLoaded=document.querySelectorAll('#line path').length>=12&&document.getElementById('line').getBoundingClientRect().width>0;
  const configurationSaved=saved.title===label&&saved.background===background&&saved.startTime===settings.value.startTime&&saved.endTime===settings.value.endTime;
  await props.host.request('storage.set',{...old,verification:{uiVerified:stylesLoaded&&imagesLoaded&&configurationSaved&&invalidSaveRejected,stylesLoaded,imagesLoaded,configurationSaved,invalidSaveRejected,savedLabel:saved.title,runtimeLoads:(previous?.runtimeLoads??0)+1,renderedText:document.querySelector('.deadline-days').textContent,origin:location.origin}});
}
</script>
<template><main class="widget" :style="theme"><div id="deadline"><div class="artwork" aria-hidden="true" v-html="artwork"/><div class="deadline-days"><div class="mask-red"><div class="inner">{{settings.title}} 剩余 <span class="day">{{progress.remainingDays}}</span> <span class="days">天</span></div></div></div></div><button class="configure" type="button" aria-label="Deadline设置" title="Deadline设置" @click="open">⚙</button><output class="status" role="status">{{host.preview?'预览 · 不保存设置':''}}</output><dialog ref="dialog" @cancel="saving&&$event.preventDefault()"><form id="deadline-form" @submit.prevent="save"><h2>Deadline设置</h2><label>标题<input v-model="draft.title" maxlength="120"></label><label>开始日期<input v-model="draft.startDate" type="date" required></label><label>最后期限<input v-model="draft.endDate" type="date" required></label><label>主色调<input v-model="draft.primaryColor" required maxlength="80" aria-label="主色调，支持透明颜色"></label><label>背景颜色<input v-model="draft.background" required maxlength="80" aria-label="背景颜色，支持透明颜色"></label><label>圆角<input v-model="draft.radius" type="number" min="0" max="50" step="1" required></label><p class="error" role="status">{{saveError}}</p><footer><button type="button" :disabled="saving" @click="dialog.close()">取消</button><button type="submit" :disabled="host.preview||saving">{{saving?'保存中':'保存'}}</button></footer></form></dialog></main></template>
