<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import IconPark from '../Icon.vue';const props=defineProps({notification:Object});const emit=defineEmits(['action']);const cancel=()=>emit('action','dismiss'),confirm=()=>emit('action',props.notification.sourceName ? 'open-source' : 'confirm');
</script>
<template>
  <div class="message-notification">
    <div class="info">
      <IconPark  class="icon" type="computer" theme="outline" />

      <div class="text">
        <div class="title">
          {{ notification.sourceName ? notification.sourceName + ' · ' + notification.title : notification.title }}
        </div>
        <div class="message">
          {{ notification.message }}
        </div>
      </div>
    </div>
    <div class="action">
      <div role="button" tabindex="0" @keydown.enter="cancel" class="button cancel" @click="cancel">
        {{ notification.sourceName ? '关闭' : '知道了' }}
      </div>
      <div role="button" tabindex="0" @keydown.enter="confirm" class="button confirm" @click="confirm">
        {{ notification.sourceName ? '打开组件' : '休息一下' }}
      </div>
    </div>
  </div>
</template>
<style scoped lang="scss">


.message-notification {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  align-items: center;
  justify-items: center;
  align-content: center;
  justify-content: space-evenly;
  padding-left: 16px;
  padding-right: 16px;

  .action {
    display: flex;
    width: 100%;

    .button {
      flex: 1;
      height: 44px;
      font-size: 18px;
      display: flex;
      align-content: center;
      justify-content: center;
      align-items: center;
      border-radius: 22px;
      font-weight: 500;
      color: white;

      &.cancel {
        background-color: #2c2c2d;
      }

      &.confirm {
        color: #5d91d9;
        background-color: #1a1c2d;
      }

      &:not(:last-child) {
        margin-right: 10px;
      }
    }
  }

  .info {
    display: flex;
    width: 100%;
    align-items: center;
    padding-left: 16px;
    padding-right: 16px;

    .icon {
      font-size: 36px;
      margin-right: 16px;
      display: flex;
      justify-items: center;
      color: #5d8ac8;
    }

    .text {
      .title {
        color: white;
        font-size: 16px;
        white-space: nowrap;
        max-width: 280px;
        overflow: hidden;
        text-overflow: ellipsis;
      }

      .message {
        color: #818383;
        font-size: 16px;
        white-space: nowrap;
        max-width: 280px;
        overflow: hidden;
        text-overflow: ellipsis;
      }
    }
  }

}


</style>
