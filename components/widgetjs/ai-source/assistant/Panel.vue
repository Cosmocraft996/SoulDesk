<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject}from'vue';import SettingHeader from'./Header.vue';import BaseWebview from'./Page.vue';import Tip from'./Tip.vue';import Settings from'./Settings.vue';const ui=inject('assistant-ui');
</script>
<template>
  <!--    <chatgpt-search-widget></chatgpt-search-widget> -->
  <div class="wrapper" >
    <!--    <DuelWebview v-if="platformUrlList.length == 2" :url1="platformUrlList[0]" :url2="platformUrlList[1]" /> -->
    <SettingHeader />
    <div class="grid" :class="[`size-${ui.state.value.config.pageCount}`]">
      <BaseWebview v-for="index in ui.state.value.config.pageCount" :key="index - 1" :index="index - 1" class="web-page" />
    </div>
    <Tip v-if="!ui.settingsOpen.value"/><Settings v-if="ui.settingsOpen.value" class="settings-overlay"/>
    <div class="background" />
  </div>
</template>
<style scoped lang="scss">
$padding: 12px;
.background {
  display: block;
  position: absolute;
  width: calc(100% - $padding * 2);
  height: calc(100% - $padding * 2);
  .background {
    border-radius: 4px;
  }
}

.wrapper {
  border-radius: 8px 0 0 8px;
  width: 100vw;
  height: 100vh;
  display: flex;
  gap: 0.5rem;
  flex-direction: column;
  position: relative;
  padding: $padding;
  background: rgba(0, 0, 0, 0.3);
}

.grid{
  display: grid;
  flex: 1;
  grid-template-columns: 1fr;
  grid-template-rows: 1fr;
  grid-column-gap: $padding;
  grid-row-gap: $padding;
  height: 100%;
  width: 100%;
  &.size-2{
    grid-template-rows: repeat(2, 1fr);
  }
  &.size-4{
    grid-template-columns: repeat(2, 1fr);
    grid-template-rows: repeat(2, 1fr);
  }
}

.web-page{
  width: 100%;
  flex: 1;
  background-color: blue;
  border-radius: 6px;
  z-index: 2;
}

 .wrapper{box-sizing:border-box;gap:12px;overflow:hidden}.grid{min-height:0;height:auto}.web-page{min-height:0;background:var(--el-bg-color,#fff)}.settings-overlay{position:absolute;inset:12px;z-index:150;overflow:auto;background:var(--el-bg-color,#fff);border-radius:6px}</style>
