<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject}from'vue';import Header from'../parts/Header.vue';import List from'../parts/List.vue';import Detail from'../parts/Detail.vue';import Settings from'../parts/Settings.vue';import Feedback from'../Feedback.vue';const ui=inject('clipboard-ui');
</script>
<template>
  <div id="clipboard" class="clipboard">
    <Header @close="ui.run(()=>ui.client.hide())" />
    <Feedback/><main>
      <Settings v-if="ui.view.value==='settings'"/><Detail v-else-if="ui.view.value==='detail'"/><List v-else/>
    </main>
  </div>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

.clipboard {
  width: calc(100vw - 24px);
  height: calc(100vh - 16px);
  background-color: theme.$app-background-color;
  border-radius: 16px;
  box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);
  margin: 8px 12px;
  display: flex;
  position: relative;
  flex-direction: column;
  color: theme.$app-text-color;
  overflow: hidden;

  main {
    display: flex;
    background-color: #f0f4f5;
    overflow: hidden;
    height: 100%;
    border-radius: 0 0 16px 16px;
    border-bottom: 2px solid theme.$app-border-color;
    border-left: 2px solid theme.$app-border-color;
    border-right: 2px solid theme.$app-border-color;
  }
}

main{min-height:0;flex:1}</style>
