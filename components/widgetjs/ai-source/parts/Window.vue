<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import GeneratorTools from'../Tools.vue';
</script>
<template>
  <div class="window">
    <slot name="title" />
    <div class="window-body">
      <GeneratorTools/><slot />
    </div>
  </div>
</template>
<style scoped lang="scss">
.window {
    box-sizing: border-box;
    width: 100vw;
    height: 100vh;
    font-size: 16px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background-color: var(--background-color);
    color: var(--color);
}
.window-body{
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
  width: 100vw;
  overflow: auto;
  height: var(--window-body-height);
}

 .window-body{flex:1;min-height:0}.window{height:100%;width:100%}.window-body{width:100%}
</style>
