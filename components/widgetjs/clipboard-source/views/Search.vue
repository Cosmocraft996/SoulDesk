<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import ActionIcon from'../parts/ActionIcon.vue';const ui=inject('clipboard-ui'),data=computed({get:()=>ui.state.searchContent,set:v=>ui.client.edit({searchContent:v})});
</script>
<template>
  <div class="hover-wrapper" @mouseenter="ui.client.hover(true)" @mouseleave="ui.client.hover(false)" :title="ui.state.error||ui.state.notice">
    <div class="content"><small v-if="ui.state.error" class="search-error" role="alert">{{ui.state.error}}</small>
      <input v-model="data" maxlength="50000" aria-label="剪切板搜索文本" @keydown.enter="ui.search()">
    </div>
    <div class="actions">
      <ActionIcon emoji="🤯" label="思维导图" @click="ui.run(()=>ui.client.tool('mindmap'))" />
      <ActionIcon emoji="🌏" label="翻译" @click="ui.run(()=>ui.client.tool('translate'))" />
      <ActionIcon emoji="🔍" label="搜索" @click="ui.search()" />
      <ActionIcon emoji="⚙️" label="设置" @click="ui.settings()" />
    </div>
  </div>
</template>
<style scoped lang="scss">
.hover-wrapper {
  height: 44px;
  display: flex;
  background-color: white;
  border-radius: 0 0 8px 8px;
  overflow: hidden;
  padding: 8px 16px;
  margin: 0 4px 0 4px;
  gap: 12px;
  align-items: center;
  box-sizing: border-box;
  border-image: linear-gradient(221deg, #a8a8a8 0%, rgba(168, 168, 168, 0) 70%) 1;
  backdrop-filter: blur(40px);
  box-shadow: 0 4px 4px 0 rgba(0, 0, 0, 0.25);

  input{
    border: none;
    flex: 1;
    width: 100%;
    font-size: 1.1rem;
    outline: none;
  }
  .content {
    white-space: nowrap;
    text-overflow: ellipsis;
    flex: 1;
    overflow: hidden;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: center;
    margin-left: auto;
    gap: 12px;

    .search-engine {
      width: 28px;
      height: 28px;
      display: flex;
      align-items: center;
      cursor: pointer;
      justify-content: center;

      img {
        width: 24px;
        height: 24px;
        background: transparent;
      }
      &:hover {
        box-sizing: border-box;
        border-radius: 4px;
        background-color: #d9d9d9;
      }
    }
  }
}

 .content{color:#161638;display:flex;flex-direction:column}.search-error{color:#b63838;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-size:10px;line-height:12px}.hover-wrapper{height:46px}</style>
