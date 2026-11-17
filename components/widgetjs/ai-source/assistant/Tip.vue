<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed,inject}from'vue';const ui=inject('assistant-ui'),t=ui.t,tipProgress=computed(()=>ui.state.value.config.tipProgress);
</script>
<template>
  <div v-if="tipProgress < 3" class="tips">
    <h3>{{tipProgress===0?t('tipTitle'):tipProgress===1?t('selectAiTitle'):t('twoLineTitle')}}</h3>
    <ol v-if="tipProgress===0">
      <li>{{ t('tipShortcut') }} <b class="blue">{{ui.state.value.config.shortcut}}</b></li>
      <li v-text="t('tipAbroad')" />
      <li>{{ t('tipRegion') }}</li>
    </ol>
    <p v-if="tipProgress>0">{{tipProgress===1?t('selectAiDesc'):t('twoLineDesc')}}</p><div class="button" :inert="ui.locked.value">
      <el-button type="warning" @click="ui.settings()">
        {{ t('tipSetting') }}
      </el-button>
      <el-button type="primary" @click="ui.guide()">
        {{ t('know') }}
      </el-button>
    </div>
  </div>
</template>
<style scoped lang="scss">
.tips{
  display: flex;
  position: absolute;
  width: calc(100% - 16px * 2);
  height: calc(100% - 16px * 2);
  background: white;
  flex-direction: column;
  font-weight: normal;
  justify-content: center;
  align-items: center;
  z-index: 100;
  gap: 1rem;
  b.blue{
    color: #0099ff
  }
  font-size: 18px;

  .button{
    display: flex;
  }

  ol{
    max-width: 500px;
  }
  h3{
    font-size: 20px;
    font-weight: 600;
    margin: 0;
    padding: 0;
  }

}

 .tips{box-sizing:border-box;inset:12px;width:auto;height:auto;padding:16px;color:var(--el-text-color-primary,#303133);background:var(--el-bg-color,#fff);font-size:14px;text-align:left}.tips ol{padding-left:24px;line-height:1.8}.tips p{line-height:1.8}</style>
