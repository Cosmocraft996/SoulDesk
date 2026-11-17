<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import Window from'../parts/Window.vue';import WindowTitleBar from'../parts/Title.vue';import EmojiLogo from'../parts/EmojiLogo.vue';import ThemeToggle from'../parts/Theme.vue';const generator=inject('ai-generator'),{context,aiChat,generate,isDark}=generator;import{Copy}from'@icon-park/vue-next';import Logo from'../original/src/assets/images/globe_showing_asia-australia_3d.png';import LanguageSelect from'../parts/Language.vue';const firstLang=computed({get:()=>context.firstLang,set:value=>context.firstLang=value}),secondLang=computed({get:()=>context.secondLang,set:value=>context.secondLang=value});const copy=()=>generator.copyText(aiChat.latestResponseContent.value);
</script>
<template>
  <Window>
    <div class="flex pl-2 items-center justify-between pr-2">
      <el-form  class="mt-2" inline :disabled="generator.loading.value">
        <el-form-item label="母语" prop="knowledge">
          <LanguageSelect v-model="firstLang" />
        </el-form-item>
        <el-form-item label="第二语言" prop="knowledge">
          <LanguageSelect v-model="secondLang" />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" @click="generate">
            翻译
          </el-button>
        </el-form-item>
      </el-form>
      <ThemeToggle />
    </div>
    <div class="flex flex-col h-full p-2 gap-2">
      <el-input v-model="context.content" type="textarea" class="flex-1 h-full" style="min-height: 150px" @keyup.enter="generate" />
      <div class="result">
        <pre>{{ aiChat.latestResponseContent }}</pre>
        <div class="btn-copy" @click="copy">
          <Copy />
        </div>
      </div>
    </div>
    <template #title>
      <WindowTitleBar title="翻译">
        <template #logo>
          <EmojiLogo :src="Logo" :glow-color="isDark ? '#d6cde1' : '#47b9ff'" />
        </template>
      </WindowTitleBar>
    </template>
  </Window>
</template>
<style scoped lang="scss">
.result{
  border: var(--surface-border);
  border-radius: 8px;
  padding: 12px;
  position: relative;
  flex: 1;
  pre{
    font-family: inherit;
    text-wrap: wrap;
  }
  .btn-copy{
    width: 24px;
    height: 24px;
    position: absolute;
    text-align: center;
    opacity: 0.3;
    line-height: 1.6;
    font-size: 18px;
    top:12px;
    padding: 4px;
    right: 12px;
    cursor: pointer;
    border-radius: 4px;
    border: var(--surface-border);
    &:hover{
      opacity: 1;
    }
  }
}

.el-textarea__inner{
  height: 100%;
}
.el-form-item{
  margin-bottom: 0.5rem;
}


</style>
