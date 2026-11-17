<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed,inject,ref,nextTick,watch,onBeforeUnmount}from'vue';
const props=defineProps({modelValue:String,index:Number}),ui=inject('assistant-ui'),AiUtils=ui.AiUtils,t=ui.t,approved=ref(false),selectRef=ref(),opening=ref(false);
let saving=Promise.resolve(),closeRequested=false,closing=false,disposed=false;
function choose(value){saving=Promise.resolve(ui.platform(props.index,value));}
const model=computed({get:()=>props.modelValue,set:choose}),selectedAiService=computed(()=>AiUtils.get(model.value)),otherOpen=computed(()=>ui.state.value.surface?.overlays?.some(id=>id.startsWith('select')&&id!=='select'+props.index));
async function prepare(){if(opening.value||ui.locked.value||otherOpen.value)return;opening.value=true;closeRequested=false;if(await ui.client.overlay('select'+props.index,true)){approved.value=true;await nextTick();selectRef.value?.focus();}opening.value=false;}
async function closeMenu(){if(closing||disposed||!approved.value)return;closing=true;try{await saving;if(disposed||ui.state.value.busy||ui.state.value.uncertain)return;if(await ui.client.overlay('select'+props.index,false))approved.value=false;}finally{closing=false;}}
function visible(value){if(!value&&approved.value){closeRequested=true;void closeMenu();}}
watch(()=>ui.state.value.uncertain,value=>{if(!value&&closeRequested)void closeMenu();});
onBeforeUnmount(()=>{disposed=true;});
</script>
<template><div class="safe-select"><button v-if="!approved" type="button" class="select-trigger" :disabled="ui.locked.value||opening||otherOpen" @click="prepare"><img :src="selectedAiService.logo" class="logo" alt="">{{opening?'正在展开…':selectedAiService.name}}<span>⌄</span></button>
  <el-select v-else ref="selectRef" automatic-dropdown :disabled="ui.locked.value" @visible-change="visible" v-model="model" class="ai-select" :placeholder="t('selectAiPlaceholder')">
    <template #label>
      <div class="flex items-center gap-2">
        <img :src="selectedAiService.logo" class="logo" alt="ai.name"> {{ selectedAiService.name }}
      </div>
    </template>
    <el-option-group :label="t('usAi')">
      <el-option v-for="ai in AiUtils.usAiServices" :key="ai.value" :label="ai.name" :value="ai.value">
        <div class="flex items-center gap-2">
          <img :src="ai.logo" class="logo" alt="ai.name"> {{ ai.name }}
        </div>
      </el-option>
    </el-option-group>
    <el-option-group :label="t('chinaAi')">
      <el-option v-for="ai in AiUtils.chinaAiServices" :key="ai.value" :label="ai.name" :value="ai.value">
        <div class="flex items-center gap-2">
          <img :src="ai.logo" class="logo" alt="ai.name"> {{ ai.name }}
        </div>
      </el-option>
    </el-option-group>
  </el-select>
</div></template>
<style scoped lang="scss">
.logo{
  width: 24px;
  height: 24px;
}

 .safe-select{height:32px}.select-trigger{height:32px;width:100%;display:flex;align-items:center;gap:8px;background:var(--el-fill-color-blank,#fff);color:var(--el-text-color-regular,#606266);border:1px solid var(--el-border-color,#dcdfe6);border-radius:4px;padding:0 8px;font:inherit;cursor:pointer}.select-trigger span{margin-left:auto}.select-trigger:disabled{cursor:wait;opacity:.7}.safe-select .el-select{width:100%}</style>
