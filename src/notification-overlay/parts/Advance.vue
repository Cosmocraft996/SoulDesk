<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{computed}from'vue';const props=defineProps({title:String,message:String,targetAt:Number,now:Number});const emit=defineEmits(['action']);const minutes=computed(()=>Math.max(0,Math.round((props.targetAt-props.now)/60000)));const onNotificationClick=()=>emit('action','countdown');
</script>
<template>
  <div class="countdown">
    <div class="content">
      <div class="title">
        {{ title }}
      </div>
      <span class="message">{{ message }}</span>
    </div>
    <button class="notification-action" type="button" @click="onNotificationClick">
      <span class="time">{{ minutes }} 分钟</span>
      <span>进入倒计时</span>
    </button>
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
  padding: 8px 2px 8px 14px;
  gap: 8px;

  .progress {
    position: absolute;
    height: 100%;
    width: 100%;
    right: 0;
    z-index: -1;
    background-image: linear-gradient(-90deg, #f6d365 0%, #fda085 100%);
  }

  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    height: 100%;
    justify-content: space-around;
    flex-direction: column;

    .title {
      font-size: 1rem;
      color: white;
    }

    .message {
      color: rgba(255, 255, 255, 0.6);
      font-size: 0.8rem;
    }

    .title, .message { max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  }

  .notification-action {
    flex: none;
    display: grid;
    padding: 5px 8px;
    border: 0;
    border-radius: 10px;
    background: #202822;
    color: #ddd;
    font-size: 11px;
    cursor: pointer;
  }

  .timing {
    font-size: 12px;
  }

  .time {
    color: #05e205;
    font-size: 1rem;
  }

  .icon {
    background-color: #ff4e4e;
    border-radius: 50%;
    padding: 2px;

    &:before {
      font-size: 1.2rem;
      color: white;
    }
  }

}


</style>
