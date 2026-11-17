<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{computed,ref,onMounted,onBeforeUnmount}from'vue';import MessageNotification from'./parts/Message.vue';import ReminderNotification from'./parts/Reminder.vue';import AdvanceCountdownNotification from'./parts/Advance.vue';import CountingNotification from'./parts/Counting.vue';import PhoneCallNotification from'./parts/Phone.vue';
const props=defineProps({host:Object,snapshot:Object});const error=ref(''),elapsed=ref(0);let timer,start=0,pointerY=0;const notification=computed(()=>({...props.snapshot.notification,type:props.snapshot.notification.kind})),now=computed(()=>props.snapshot.sampledAt+elapsed.value);let sample=props.snapshot.sampledAt;
async function action(value){if(props.host.preview)return;error.value='';try{await props.host.request('island.action',{id:notification.value.id,action:value});}catch(e){error.value=e.message;}}
async function hover(entered){if(props.host.preview)return;try{await props.host.request('island.hover',{id:notification.value.id,entered});}catch(e){error.value=e.message;}}
function down(event){pointerY=event.clientY;}function up(event){if(event.clientY-pointerY< -35)void action('dismiss');}
onMounted(()=>{start=performance.now();timer=setInterval(()=>{if(sample!==props.snapshot.sampledAt){sample=props.snapshot.sampledAt;start=performance.now();}elapsed.value=performance.now()-start;},10);});onBeforeUnmount(()=>clearInterval(timer));
</script>
<template>
  <div class="island" :class="notification.type" @mouseenter="hover(true)" @mouseleave="hover(false)" @pointerdown="down" @pointerup="up">
    <CountingNotification v-if="notification.type==='countdown'" v-bind="notification" :now="now" />
    <AdvanceCountdownNotification v-else-if="notification.type==='advance-countdown'" v-bind="notification" :now="now" @action="action" />
    <ReminderNotification v-else-if="notification.type==='reminder'" :notification="notification" @action="action" />
    <PhoneCallNotification v-else-if="notification.type==='call'" :key="notification.id" :host="host" :notification="notification" :displayed="snapshot.displayed" @action="action" @error="error=$event" />
    <MessageNotification v-else :type="notification.type" :message="notification.message" />
    <span v-if="error" class="display-error" role="alert">{{ error }}</span>
  </div></template>
<style scoped lang="scss">
$cubic-bezier: cubic-bezier(0, 1, 0.68, 1.05);
.island {
  -webkit-font-smoothing: antialiased;
  user-select: none;
  cursor: pointer;
  border-radius: 42px;
  display: flex;
  font-size: 2px;
  top: 0;
  left: 8px;
  right: 0;
  margin: auto;
  position: absolute;
  height: calc(100% - 16px);
  background-color: black;
  will-change: width, height;
  clip: auto;
  overflow: hidden;
  transition-property: width, height;
  justify-content: flex-start;
  box-shadow: (0px 4px 4px 4px rgba(0, 0, 0, 0.04), 0px 4px 6px rgba(0, 0, 0, 0.08));

  .mask {
    position: absolute;
    width: 100%;
    height: 100%;
    z-index: 5;
    left: 0;
    top: 0;
    background-color: black;
  }
}

.island{width:326px;left:0;transition:transform .2s,height .2s}.island.info,.island.success,.island.error,.island.warning,.island.countdown{width:310px}.display-error{position:absolute;bottom:2px;color:#ff9090;font-size:10px;max-width:90%;overflow:hidden;white-space:nowrap}
</style>
