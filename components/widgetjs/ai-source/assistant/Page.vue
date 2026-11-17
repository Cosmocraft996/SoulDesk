<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed,inject}from'vue';import AiSelect from'./Select.vue';const props=defineProps({index:{type:Number,default:0}}),ui=inject('assistant-ui'),page=computed(()=>ui.state.value.surface?.pages?.[props.index]),aiService=computed({get:()=>ui.state.value.config.platforms[props.index],set:v=>ui.platform(props.index,v)});
</script>
<template>
  <div style="position: relative">
    <div class="native-page" :data-slot="index" :aria-label="ui.AiUtils.get(aiService).name+'网页区域'"><p>{{page?.phase==='pending'?'等待打开网页':page?.phase==='loading'?'正在载入网页':page?.phase==='failed'?'网页载入失败':'网页区域'}}</p></div>
    <small class="page-state" :title="page?.error||''">{{page?.phase==='loading'?'载入中':page?.phase==='failed'?'载入失败':page?.phase==='ended'?'载入结束':''}}</small>
    <AiSelect v-model="aiService" :index="index" class="ai-selector" />
  </div>
</template>
<style scoped lang="scss">
.ai-selector{
  position: absolute;
  bottom: 8px;
  transform: translateX(-50%);
  left: 50%;
  width: 150px;
  z-index: 999;
  opacity: 0.8;
  transition: opacity 0.4s ease;
  &:hover{
    opacity: 1;
  }
}
webview{
  width: 100%;
  border-radius: 6px;
  height: 100%;
  overflow: hidden;
  background-color: white;
}

@media (prefers-color-scheme: dark) {
  webview{
    background: #292a2d;
  }
}

 .native-page{position:absolute;inset:0 0 40px;border-radius:6px;background:var(--el-bg-color,#fff);display:grid;place-items:center;color:var(--el-text-color-secondary,#909399)}.page-state{position:absolute;bottom:12px;right:6px;max-width:calc(50% - 80px);font-size:10px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}</style>
