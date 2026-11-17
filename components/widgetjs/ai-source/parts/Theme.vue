<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import{Laptop,Moon,Sun}from'@icon-park/vue-next';const generator=inject('ai-generator'),mode=generator.theme;function toggleTheme(){mode.value=mode.value==='auto'?'light':mode.value==='light'?'dark':'auto';}
</script>
<template>
  <button class="theme-toggle" :title="`Toggle Theme (${mode})`" @click="toggleTheme()">
    <Laptop v-if="mode === 'auto'" size="18" />
    <Moon v-else-if="mode === 'dark'" size="18" />
    <Sun v-else size="18" />
  </button>
</template>
<style scoped lang="scss">
.theme-toggle {
  display: flex;
  justify-content: center;
  align-items: center;
  width: 24px;
  height: 24px;
  cursor: pointer;
  color: var(--el-text-color-primary);
  transition: color 0.3s;

  &:hover {
    color: var(--el-color-primary);
  }
}


</style>
