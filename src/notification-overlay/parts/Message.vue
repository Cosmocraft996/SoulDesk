<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import IconPark from '../Icon.vue';const props=defineProps({type:String,message:String,title:String,source:String});
</script>
<template>
  <div class="message-notification ">
    <IconPark :class="`icon ${props.type}`" :type="type" theme="outline" />
    <div v-if="source" class="copy" :title="source + ' · ' + title + (message ? ' — ' + message : '')">
      <strong>{{ source }} · {{ title }}</strong>
      <span v-if="message">{{ message }}</span>
    </div>
    <div v-else class="message">
      {{ message }}
    </div>
  </div>
</template>
<style scoped lang="scss">


.message-notification {
  display: flex;
  width: 100%;
  height: 100%;
  align-items: center;
  padding-left: 16px;
  padding-right: 16px;

  .message {
    color: white;
    font-size: 16px;
    white-space: nowrap;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .copy {
    min-width: 0;
    display: grid;
    color: white;

    strong, span {
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    strong { font-size: 14px; }
    span { font-size: 12px; color: #d4d4d8; }
  }

  .icon {
    font-size: 1.5rem;
    margin-right: 0.5rem;

    &.success {
      color: #67C23A;
    }

    &.error {
      color: #F56C6C;
    }

    &.warning {
      color: #E6A23C;
    }

    &.info {
      color: #909399;
    }

  }
}


</style>
