<script setup>
import {ref,onMounted,onBeforeUnmount} from 'vue';
const props=defineProps({host:{type:Object,required:true}});
const status=ref({enabled:false,listening:false,suspended:false,lastError:null}),error=ref(''),busy=ref(false);
let timer,disposed=false,generation=0;
async function refresh(){
  if(busy.value||props.host.preview||disposed)return;
  const at=generation;
  try{const next=await props.host.request('keyboard.status');if(!disposed&&!busy.value&&generation===at){status.value=next;error.value='';}}catch(cause){if(!disposed&&generation===at)error.value=String(cause.message||cause);}
}
async function configure(enabled){
  if(busy.value||props.host.preview)return;
  generation++;busy.value=true;error.value='';
  try{const next=await props.host.request('keyboard.configure',{enabled});if(!disposed)status.value=next;}
  catch(cause){if(!disposed)error.value=String(cause.message||cause);}
  finally{busy.value=false;}
}
onMounted(()=>{void refresh();timer=setInterval(refresh,1000);});
onBeforeUnmount(()=>{disposed=true;clearInterval(timer);});
</script>
<template>
  <main class="control">
    <div class="heading"><span class="keycap">Ctrl</span><span>＋</span><span class="keycap">C</span><strong>键盘演示</strong></div>
    <p>在屏幕底部显示修饰键组合和 CapsLock，松开约4秒后隐藏。</p>
    <button :disabled="busy||host.preview" :aria-pressed="status.enabled" @click="configure(!status.enabled)">{{busy?'正在保存…':status.enabled?'停用键盘演示':'启用键盘演示'}}</button>
    <button v-if="status.enabled&&status.lastError" :disabled="busy" @click="configure(true)">重试</button>
    <small v-if="host.preview">预览不会启用键盘监听</small>
    <small v-else-if="status.suspended">已启用；隐藏全部期间暂停</small>
    <small v-else-if="status.listening">正在演示快捷键</small>
    <small v-else>{{status.enabled?'已启用，正在准备叠加层…':'默认关闭；仅在你启用后监听'}}</small>
    <p class="error" v-if="error||status.lastError" role="alert">{{error||status.lastError}}</p>
  </main>
</template>
