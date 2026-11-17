<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{computed}from'vue';import{clockText,progress as percentage}from'../client.js';const props=defineProps({message:String,targetAt:Number,createdAt:Number,now:Number});const timeString=computed(()=>clockText(props.targetAt-props.now)),progress=computed(()=>percentage(props.targetAt-props.now,props.targetAt-props.createdAt));const onNotificationClick=()=>{};
</script>
<template>
  <div class="countdown" @click="onNotificationClick">
    <div class="content">
      <div class="title">
        {{ message }}
      </div>
    </div>
    <span class="timing">{{ timeString }}</span>
    <div class="progress">
      <div class="progress-bar" :style="{ width: `${progress}%` }" />
    </div>
  </div>
</template>
<style scoped lang="scss">
$easing: cubic-bezier(1, 0, 1, 0);
.countdown {
  width: 100%;
  display: flex;
  flex-direction: row;
  align-items: center;
  position: relative;
  padding: 1rem 0.8rem;

  .progress {
    position: absolute;
    height: 100%;
    width: 100%;
    left: 0;
    border-radius: 24px;

    .progress-bar {
      position: absolute;
      height: 100%;
      right: 0;
      transition-duration: 0.5s;
      border-radius: 24px;
      transition-property: width;
      background-image: linear-gradient(-60deg, #ff5858 0%, #f09819 100%);
    }
  }

  .content {
    flex: 1;
    display: flex;
    align-items: flex-start;
    height: 100%;
    justify-content: space-around;
    flex-direction: column;
    z-index: 2;

    .title {
      font-size: 1rem;
      color: white;
    }

  }

  .timing {
    z-index: 2;
    font-size: 1rem;
    color: white;
  }

}


</style>
