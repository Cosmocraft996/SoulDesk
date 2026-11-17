<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import{Clear,Close,SettingTwo,Refresh,Star}from'@icon-park/vue-next';import SearchBox from'./SearchBox.vue';const emits=defineEmits(['close']),ui=inject('clipboard-ui'),keyword=computed({get:()=>ui.state.keyword,set:v=>ui.run(()=>ui.client.setQuery(v))});
</script>
<template>
  <header>
    <SearchBox v-model="keyword" />
    <Refresh title="刷新" @click="ui.run(()=>ui.client.refresh(true))"/><Star title="只看收藏" :theme="ui.state.favoritesOnly?'filled':'outline'" @click="ui.run(()=>ui.client.setQuery(ui.state.keyword,!ui.state.favoritesOnly))"/>
    <el-popconfirm
      title="确定清空所有未收藏记录？"
      width="200"
      cancel-button-text="取消"
      confirm-button-text="确定"
      @confirm="ui.run(()=>ui.client.clear())"
    >
      <template #reference>
        <Clear />
      </template>
    </el-popconfirm>
    <span title="设置" @click="ui.navigate('settings')">
      <SettingTwo theme="outline" />
    </span>
    <Close style="margin-left: auto" @click="emits('close')" />
  </header>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

header {
  padding: 8px 12px;
  display: flex;
  background-color: #6365ff;
  justify-items: center;
  align-items: center;
  gap: 0.5rem;
  border-radius: theme.$app-border-radius theme.$app-border-radius 0 0;
  box-shadow: inset 0px 1px 0px 0px rgba(255, 255, 255, 0.3);

  .i-icon {
    color: white;
    cursor: pointer;
    font-size: 1.2em;
    padding: 4px;
    border-radius: 4px;
    transition: background-color 0.2s ease-in-out;

    &:hover {
      background-color: rgba(255, 255, 255, 0.23);
    }
  }
}

</style>
