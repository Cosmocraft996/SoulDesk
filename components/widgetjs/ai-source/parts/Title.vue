<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
defineProps({title:String});
</script>
<template>
  <div class="window-title-bar">
    <slot name="title">
      <div class="title">
        <slot name="logo" />
        <div class="title">
          {{ title }}
        </div>
      </div>
    </slot>
    <div class="actions" style="margin-left: auto; display: flex; align-items: center; -webkit-app-region: no-drag;">
      <slot name="actions" />
    </div>

  </div>
</template>
<style scoped lang="scss">
.window-title-bar{
  display: flex;
  width: 100vw;
  height: var(--window-title-bar-height);
  padding: 6px 16px;
  background: var(--surface-background);
  box-sizing: border-box;
  color: var(--color);
  font-weight: bold;
  z-index: 2;
  app-region: drag;
  align-items: center;
  border-bottom: var(--surface-border);
  .title{
    display: flex;
    gap: 0.5rem;
    justify-items: center;
    align-items: center;
  }
  .buttons{
    display: flex;
    gap:8px;
  }
  .button {
    cursor: pointer;
    width: 18px;
    height: 18px;
    box-sizing: border-box;
    display: flex;
    justify-content: center;
    align-items: center;
    border-radius: 50%;
    app-region: no-drag;
    color: white;
    transition: background 0.3s;
    &.minimize{
      background: #f2b63e;
      &:hover {
        filter: drop-shadow(0 2px 5px #f2b63e);
      }
    }
    &.close{
      background: #ed4f4a;
      &:hover {
        filter: drop-shadow(0 2px 5px #ed4f4a);
      }
    }
    &.maximize{
      background: #5adc41;
      &:hover {
        filter: drop-shadow(0 2px 5px #5adc41);
      }
    }
  }
}

 .window-title-bar{width:100%;flex-shrink:0;-webkit-app-region:no-drag}
</style>
