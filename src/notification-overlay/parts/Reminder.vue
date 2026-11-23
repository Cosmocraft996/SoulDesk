<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import IconPark from '../Icon.vue';
const props = defineProps({ notification: Object });
const emit = defineEmits(['action']);
const hasSource = () => props.notification.sourceName;
const cancel = () => emit('action', 'dismiss');
const confirm = () => emit('action', hasSource() ? 'open-source' : 'confirm');
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
      <button type="button" class="button cancel notification-action" @click="cancel">
        {{ hasSource() ? '关闭' : '知道了' }}
      </button>
      <button type="button" class="button confirm notification-action" @click="confirm">
        {{ hasSource() ? '打开组件' : '休息一下' }}
      </button>
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
      border: 0;
      cursor: pointer;
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
