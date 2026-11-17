<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{ref,watch}from'vue';const props=defineProps({text:String,fontSize:Number});const text1=ref(props.text),text2=ref(''),toggle=ref(true);watch(()=>props.text,value=>{toggle.value=!toggle.value;if(toggle.value)text1.value=value;else text2.value=value;});
</script>
<template>
  <div class="text-switcher">
    <!--    定位宽度用   -->
    <span :style="{ fontSize: `${fontSize}px`, visibility: 'hidden' }">{{ text }}</span>
    <span ref="textSpan1" class="text-1" :class="{active:toggle}" :style="{ fontSize: `${fontSize}px` }">{{ text1 }}</span>
    <span ref="textSpan2" class="text-2" :class="{active:!toggle}" :style="{ fontSize: `${fontSize}px` }">{{ text2 }}</span>
  </div>
</template>
<style scoped lang="scss">
.text-switcher {
  position: relative;

  .text-2, .text-1 {
    position: absolute;
    left: 0;
    top: 0;
  }
}

.text-1,.text-2{opacity:0;transform:translateY(-20px);transition:opacity .3s,transform .3s}.text-1.active,.text-2.active{opacity:1;transform:translateY(0)}
</style>
