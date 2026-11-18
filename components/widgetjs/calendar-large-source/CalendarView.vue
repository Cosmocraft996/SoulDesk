<script setup>
import{nextTick,onBeforeUnmount,onMounted,ref,watch}from'vue';
import{Calendar,dayGridPlugin,rrulePlugin,interactionPlugin}from'./fullcalendar.js';
import{lunarEvents,publicEvents,todoEvents,synchronizeTodos}from'./calendar-large-model.js';
import{localDate}from'../todo-source/todo-model.js';
import{readAlmanac}from'../calendar-source/content-client.js';
const props=defineProps({host:Object,snapshot:Object,settings:Object}),emit=defineEmits(['add','edit']);
const target=ref(),sourceError=ref(''),publicCount=ref(0),publicApiPassed=ref(false),loading=ref(false);
let calendar,observer,alive=true,inFlight=null,lastContext=0,timer,day=localDate(Date.now());
const locale={code:'zh-cn',week:{dow:1,doy:4},buttonText:{prev:'上月',next:'下月',today:'今天',month:'月',week:'周',day:'日',list:'日程'},weekText:'周',allDayText:'全天',moreLinkText:n=>'另外 '+n+' 项',noEventsText:'没有日程'};
function updateTodos(){if(!calendar)return;synchronizeTodos(calendar,props.snapshot);}
function openDate(info){if(lastContext+200>Date.now())return;const value=typeof info==='string'?info:info.dateStr;const key=/^\d{4}-\d\d-\d\d/.test(value)?value.slice(0,10):localDate(info.date);emit('add',key);}
function openEvent(info){const event=info.event||info;if(event.id&&event.extendedProps.category==='default')emit('edit',event.id);}
function refresh(forceRefresh=false){if(props.host.preview||!alive)return Promise.resolve();if(inFlight)return inFlight;loading.value=true;inFlight=(async()=>{try{const response=await readAlmanac(props.host,forceRefresh),events=publicEvents(response);if(alive){calendar.batchRendering(()=>{for(const event of calendar.getEvents())if(event.extendedProps.category==='public')event.remove();for(const event of events)calendar.addEvent(event);});publicCount.value=response.data.data.length;publicApiPassed.value=true;sourceError.value='';}}catch(reason){if(alive)sourceError.value='公开节气读取失败'+(publicApiPassed.value?'，保留上次数据':'')+'：'+String(reason?.message||reason);}finally{loading.value=false;inFlight=null;}})();return inFlight;}
watch(()=>JSON.stringify(todoEvents(props.snapshot)),updateTodos);
watch(()=>props.settings.firstDayOfWeek,value=>calendar?.setOption('firstDay',value));
function dispose(){alive=false;clearInterval(timer);observer?.disconnect();calendar?.destroy();calendar=null;}
onBeforeUnmount(dispose);onMounted(async()=>{calendar=new Calendar(target.value,{plugins:[dayGridPlugin,rrulePlugin,interactionPlugin],locale,weekNumbers:false,firstDay:props.settings.firstDayOfWeek,initialView:'dayGridMonth',height:'100%',handleWindowResize:true,eventSources:[{id:'lunar',events(info,success,failure){try{success(lunarEvents(info.start,info.end));}catch(reason){failure(reason);}}}],eventClick:openEvent,dateClick:openDate});calendar.render();updateTodos();await nextTick();if(!alive)return;observer=new ResizeObserver(()=>calendar?.updateSize());observer.observe(target.value);void refresh();timer=setInterval(()=>{const next=localDate(Date.now());if(next!==day){day=next;calendar?.refetchEvents();void refresh(true);}},30000);});
defineExpose({getApi:()=>calendar,refresh,openDate,openEvent,publicCount,publicApiPassed,sourceError});
</script>
<template><section class="calendar-view"><div ref="target" class="calendar-grid calendar" @contextmenu="lastContext=Date.now()"></div><output v-if="host.preview||loading||sourceError" class="calendar-source-status" role="status">{{host.preview?'预览 · 不读取在线日历':loading?'正在读取公开节气':sourceError}}<button v-if="sourceError" type="button" @click="refresh(true)">重试</button></output></section></template>
