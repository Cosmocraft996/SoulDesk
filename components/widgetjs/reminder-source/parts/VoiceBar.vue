<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{ref,onMounted,onBeforeUnmount}from'vue';defineProps({backgroundColor:{type:String,default:'#86D98A'}});const foregroundHeight=ref(100),backgroundHeight=ref(100);let timer;onMounted(()=>{timer=setInterval(()=>{foregroundHeight.value=10+Math.random()*40;backgroundHeight.value=10+Math.random()*90;},200);});onBeforeUnmount(()=>clearInterval(timer));
</script>
<template>
  <div class="voice-bar">
    <span class="background" :style="{ transform: `scaleY(${backgroundHeight}%)`, backgroundColor }" />
    <span class="foreground" :style="{ transform: `scaleY(${foregroundHeight}%)` }" />
  </div>
</template>
<style scoped lang="scss">
.voice-bar {
  position: relative;
  width: 2px;
  height: 20px;

  .foreground, .background {
    width: 100%;
    border-radius: 6px;
    position: absolute;
    height: 100%;
    left: 0;
    top: 0;
    transition-property: transform;
    transition-duration: 0.5s;
  }

  .foreground {
    width: 2px;
    background-color: white;
  }

  .background {
    background-color: #86D98A;
  }
}


</style>
