<script setup>
import{provide}from'vue';import{useAssistant}from'./use-assistant.js';import Panel from'./assistant/Panel.vue';import Settings from'./assistant/Settings.vue';import robot from'./original/src/assets/robot.png';
const props=defineProps({host:Object,shell:{type:Boolean,default:undefined}});
const shell=props.shell??(typeof location!=='undefined'&&new URLSearchParams(location.search).get('role')==='assistant-shell');
const ui=useAssistant(props.host,shell);provide('assistant-ui',ui);
</script>
<template>
  <div class="assistant-root">
    <Panel v-if="shell"/>
    <Settings v-else-if="ui.settingsOpen.value"/>
    <section v-else class="assistant-control">
      <header><img :src="robot" alt=""><div><h2>网页 AI 助手</h2><small>{{ui.phase.value}}</small></div></header>
      <p>{{ui.state.value.config.pageCount}} 个分屏 · {{ui.state.value.config.position==='left'?'左侧':'右侧'}}窗口</p>
      <div class="selected-sites"><span v-for="(platform,index) in ui.state.value.config.platforms.slice(0,ui.state.value.config.pageCount)" :key="index"><img :src="ui.AiUtils.get(platform).logo" alt="">{{ui.AiUtils.get(platform).name}}</span></div>
      <p v-if="ui.error.value" role="alert">{{ui.error.value}}</p>
      <p v-if="ui.host.preview">预览仅展示控制卡，不加载网站或注册快捷键。</p>
      <div class="control-actions"><el-button type="primary" :disabled="ui.locked.value" @click="ui.client.action('open')">打开助手</el-button><el-button :disabled="ui.locked.value" @click="ui.settings()">设置</el-button><el-button v-if="ui.state.value.surface||ui.state.value.nativeCleanupPending" :disabled="ui.locked.value" @click="ui.client.action('hide')">收起</el-button><el-button v-if="ui.state.value.surface?.phase==='failed'||ui.state.value.nativeError" :disabled="ui.locked.value" @click="ui.client.action('retry')">重试显示</el-button></div>
      <p class="control-note">各平台由你手动登录。后台入口{{ui.state.value.config.backgroundEnabled?'已启用':'未启用'}}，{{ui.state.value.shortcutRegistered?'快捷键已注册':'可在设置中调整快捷键和代理'}}。</p>
      <p v-if="ui.state.value.shortcutError" role="alert">{{ui.state.value.shortcutError}}</p>
    </section>
  </div>
</template>
<style scoped>
.assistant-root{width:100%;height:100%;box-sizing:border-box;overflow:auto;color:var(--el-text-color-primary,#303133);background:var(--el-bg-color,#fff);font-family:system-ui,sans-serif}.assistant-control{padding:18px}.assistant-control header{display:flex;align-items:center;gap:10px}.assistant-control header>img{width:36px}.assistant-control h2{font-size:18px;margin:0 0 4px}.assistant-control small{color:var(--el-text-color-secondary,#909399)}.selected-sites{display:flex;gap:12px;flex-wrap:wrap;margin:16px 0}.selected-sites span{display:flex;align-items:center;gap:6px;font-size:13px}.selected-sites img{width:22px;height:22px;object-fit:contain}.control-actions{display:flex;flex-wrap:wrap;gap:8px}.control-actions .el-button{margin:0}.control-note{font-size:12px;line-height:1.7;color:var(--el-text-color-secondary,#909399)}[role=alert]{color:var(--el-color-danger,#f56c6c);line-height:1.6;font-size:13px;overflow-wrap:anywhere}
</style>
