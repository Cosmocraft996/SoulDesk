<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import{GridTwo,LayoutTwo,Pin,Pushpin,Setting,WebPage,Close}from'@icon-park/vue-next';import robot from'../original/src/assets/robot.png';const ui=inject('assistant-ui'),t=ui.t,pageCount=computed(()=>ui.state.value.config.pageCount),alwaysTop=computed(()=>ui.state.value.config.alwaysTop);
</script>
<template>
  <div class="setting-header">
    <div class="banner" >
      <img :src="robot">{{ t('aiAssistant') }}
      <small v-if="ui.error.value" class="header-error" role="alert" :title="ui.error.value">{{ui.error.value}}</small>
  </div>
    <div v-if="ui.state.value.uncertain||ui.state.value.surface?.phase==='failed'||ui.state.value.nativeError" class="ml-auto"><el-button style="width:auto" :disabled="ui.state.value.busy" @click="ui.state.value.uncertain?ui.client.retrySave():ui.client.action('retry')">{{ui.state.value.uncertain?'重试保存':'重试显示'}}</el-button><el-button title="收起助手" @click="ui.client.action('hide')"><Close/></el-button></div><div v-else class="ml-auto" :inert="ui.locked.value||ui.settingsOpen.value||ui.state.value.surface?.overlays?.some(id=>id.startsWith('select'))">
      <el-tooltip :content="t('singlePage')">
        <el-button :title="t('singlePage')" :aria-label="t('singlePage')" :type="pageCount == 1 ? 'primary' : ''" @click="ui.client.quick({pageCount:1})">
          <WebPage />
        </el-button>
      </el-tooltip>
      <el-tooltip :content="t('twoLine')">
        <el-button :title="t('twoLine')" :aria-label="t('twoLine')" id="two-line" :type="pageCount == 2 ? 'primary' : ''" @click="ui.client.quick({pageCount:2})">
          <LayoutTwo />
        </el-button>
      </el-tooltip>
      <el-tooltip :content="t('fourGrid')">
        <el-button :title="t('fourGrid')" :aria-label="t('fourGrid')" :type="pageCount == 4 ? 'primary' : ''" @click="ui.client.quick({pageCount:4})">
          <GridTwo />
        </el-button>
      </el-tooltip>
      <el-tooltip :content="t('setting')">
        <el-button :title="t('setting')" :aria-label="t('setting')" @click="ui.settings()">
          <Setting />
        </el-button>
      </el-tooltip>
      <el-tooltip :content="t('alwaysTop')">
        <el-button :title="t('alwaysTop')" :aria-label="t('alwaysTop')" :type="alwaysTop ? 'primary' : ''" @click="ui.client.quick({alwaysTop:!alwaysTop})">
          <Pushpin v-if="alwaysTop" /><Pin v-else />
        </el-button>
      </el-tooltip><el-button title="收起助手" @click="ui.client.action('hide')"><Close/></el-button>
    </div>
  </div>
</template>
<style scoped lang="scss">
.setting-header{
  z-index: 2;
  align-items: center;
  display: flex;
  .banner{
    display: flex;
    align-items: center;
    color: white;
    cursor: pointer;
    gap: 0.5rem;
    font-weight: bold;
    font-size: 1rem;
    img{
      width: 24px;
    }
  }

}

 .setting-header{position:relative;height:32px;flex:none;min-width:0}.banner{font-size:14px!important;gap:4px!important;white-space:nowrap}.banner img{width:20px!important}.ml-auto{display:flex;align-items:center;margin-left:auto}.ml-auto .el-button{height:28px;width:28px;padding:4px;margin-left:4px}.header-error{position:absolute;top:32px;left:0;right:0;color:#ffb4ab;font-size:10px;line-height:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}</style>
