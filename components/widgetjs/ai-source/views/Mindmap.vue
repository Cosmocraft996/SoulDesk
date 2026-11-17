<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import Window from'../parts/Window.vue';import WindowTitleBar from'../parts/Title.vue';import EmojiLogo from'../parts/EmojiLogo.vue';import ThemeToggle from'../parts/Theme.vue';const generator=inject('ai-generator'),{context,aiChat,generate,isDark}=generator;import ExplodingHead from'../original/src/assets/images/exploding_head_3d.png';import MarkdownMap from'../parts/MarkdownMap.vue';import ContextMenu from'../parts/ContextMenu.vue';const{currentText,onNodeClick,copyNode:copy,expandNode:view}=generator;
</script>
<template>
  <Window title="思维导图">
    <div class="p-2 flex-row align-center justify-between">
      <el-form  class="mt-2" inline :disabled="generator.loading.value">
        <el-form-item label="内容" prop="knowledge">
          <el-input v-model="context.content" maxlength="10000" @keyup.enter="generate" />
        </el-form-item>
        <el-form-item label="其他说明">
          <el-input v-model="context.extra" maxlength="500" />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="aiChat.sending.value" @click="generate">
            生成
          </el-button>
        </el-form-item>
      </el-form>
      <ThemeToggle />
    </div>
    <ContextMenu  @copy="copy" @view="view" />
    <MarkdownMap  :markdown="aiChat.latestResponseContent.value" @node-click="onNodeClick" />
    <template #title>
      <WindowTitleBar title="思维导图">
        <template #logo>
          <EmojiLogo :src="ExplodingHead" glow-color="rgb(243, 127, 89)" />
        </template>
      </WindowTitleBar>
    </template>
  </Window>
</template>
<style scoped lang="scss">
.mind-map-wrapper{
  flex: 1;
}

.el-form-item{
  margin-bottom: 0.5rem;
}


</style>
