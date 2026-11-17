<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';const emits=defineEmits(['copy','view']),generator=inject('ai-generator'),show=computed(()=>generator.menu.visible),leftPX=computed(()=>generator.menu.left+'px'),topPX=computed(()=>generator.menu.top+'px');
</script>
<template>
  <div v-show="show" class="context-menu" @pointerdown.stop @click="generator.menu.visible=false">
    <div class="flex flex-col gap-2">
      <div class="context-menu-item" @click="emits('copy')">
        复制
      </div>
      <div class="context-menu-item" @click="emits('view')">
        查看
      </div>
    </div>
  </div>
</template>
<style scoped lang="scss">
.context-menu{
  position: fixed;
  top: v-bind(topPX);
  left: v-bind(leftPX);
  z-index: 10;
  padding: 8px;
  border: var(--surface-border);
  background: #e0e0e0;
  border-radius: 4px;
  .context-menu-item{
    cursor: pointer;
  }
}


</style>
