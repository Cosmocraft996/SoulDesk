<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import Window from'../parts/Window.vue';import WindowTitleBar from'../parts/Title.vue';import EmojiLogo from'../parts/EmojiLogo.vue';import ThemeToggle from'../parts/Theme.vue';const generator=inject('ai-generator'),{context,aiChat,generate,isDark}=generator;import Logo from'../original/src/assets/images/bar_chart.png';import MermaidChart from'../parts/MermaidChart.vue';const{t,chartTypes}=generator,chartType=computed({get:()=>context.chartType,set:value=>context.chartType=value});
</script>
<template>
  <Window>
    <div class="flex pl-2 items-center justify-between pr-2">
      <el-form  class="mt-2" inline :disabled="generator.loading.value">
        <el-form-item :label="t('content')">
          <el-input v-model="context.content" @keyup.enter="generate" />
        </el-form-item>
        <el-form-item :label="t('chartType')">
          <el-select v-model="chartType" style="width: 150px">
            <el-option v-for="item in chartTypes" :key="item.value" :label="item.name" :value="item.value" />
          </el-select>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" @click="generate">
            {{ t('generate') }}
          </el-button>
        </el-form-item>
      </el-form>
      <ThemeToggle />
    </div>
    <div class="flex flex-1 p-2 gap-2">
      <el-input v-model="aiChat.latestResponseContent.value" type="textarea" class="flex-1 h-full" style="min-height: 150px" />
      <MermaidChart :code="aiChat.latestResponseContent.value" />
    </div>
    <template #title>
      <WindowTitleBar :title="t('mermaidChart')">
        <template #logo>
          <EmojiLogo :src="Logo" :glow-color="isDark ? '#d6cde1' : ''" />
        </template>
        <template #actions />
      </WindowTitleBar>
    </template>
  </Window>
</template>
<style scoped lang="scss">
.el-textarea__inner{
  height: 100%;
}
.el-form-item{
  margin-bottom: 0.5rem;
}


</style>
