<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{computed,ref,onMounted,onBeforeUnmount}from'vue';import TextSwitcher from'./TextSwitcher.vue';import{clockText}from'./client.js';
const props=defineProps({host:Object,snapshot:Object});const error=ref(''),view=ref();let busy=false;
const array=computed(()=>clockText(props.snapshot.remainingSeconds*1000).slice(0,-3).split(''));
async function close(){if(busy)return;busy=true;error.value='';try{await props.host.request('sit.break.close',{id:props.snapshot.id});}catch(e){error.value=e.message;}finally{busy=false;}}
const escape=event=>{if(event.key==='Escape'){event.preventDefault();void close();}};onMounted(()=>{view.value?.focus();window.addEventListener('keydown',escape);});onBeforeUnmount(()=>window.removeEventListener('keydown',escape));
</script>
<template>
  <div ref="view" class="break-view" tabindex="-1">
    <div class="countdown">
      <template v-for="(item,index) in array" :key="index">
        <TextSwitcher :text="item" :font-size="item === ':' ? 150 : 300" />
      </template>
    </div>
    <p>按Esc退出</p><button @click="close">结束休息</button><p v-if="error" role="alert">{{ error }}</p>
  </div>
</template>
<style scoped lang="scss">
.break-view {
  color: white;
  background-color: rgba(0, 0, 0, 0.8);
  position: absolute;
  width: 100%;
  height: 100%;
  display: flex;
  justify-items: center;
  justify-content: center;
  align-content: center;
  align-items: center;
  flex-direction: column;

  .text-switcher {
    color: white;
  }

  .countdown {
    display: flex;
    flex-direction: row;
    align-items: center;
    letter-spacing: 16px;
    font-weight: bold;
    outline: none;

    .number {
      font-size: 258px;
    }

    .colon {
      font-size: 128px;
    }
  }

}

.break-view{outline:none}.break-view button{font:inherit;color:white;background:#343434;border:1px solid #666;border-radius:8px;padding:8px 16px}.break-view .countdown{font-size:clamp(30px,12vw,300px)}.break-view :deep(.text-switcher span){font-size:clamp(30px,12vw,300px)!important}
</style>
