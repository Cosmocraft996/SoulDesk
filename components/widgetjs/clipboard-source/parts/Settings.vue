<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,reactive,watch}from'vue';const ui=inject('clipboard-ui'),draft=reactive(JSON.parse(JSON.stringify(ui.state.status)));watch(()=>ui.state.settingsSavedRevision,revision=>{if(Number.isSafeInteger(revision))draft.configRevision=revision;});async function save(){await ui.run(()=>ui.client.saveSettings(draft));}
</script>
<template>
  <div class="settings">
    <el-card shadow="never" style="--el-color-primary: #6365ff">
      <el-form @submit.prevent="save" :disabled="ui.host.preview||ui.locked.value"><el-form-item label="后台采集文本"><el-switch v-model="draft.captureEnabled"/></el-form-item><el-form-item label="启用全局快捷键"><el-switch v-model="draft.shortcutEnabled"/></el-form-item><el-form-item v-if="ui.kind==='history'" label="选择后粘贴回原应用"><el-switch v-model="draft.pasteEnabled"/></el-form-item>
        <el-form-item label="快捷键">
          <el-input v-model="draft.shortcut" :clearable="false" placeholder="Super+Alt+V"/>
        </el-form-item>
        <el-form-item v-if="ui.kind==='search'" label="搜索平台"><el-select v-model="draft.searchPlatform"><el-option value="google" label="Google"/><el-option value="bing" label="Bing"/><el-option value="baidu" label="百度"/></el-select></el-form-item><el-tooltip content="记录数过多可能导致卡顿、加载缓慢">
          <el-form-item label="记录保留时长：">
            <el-radio-group v-model="draft.historyExpiredHours">
              <el-radio :value="12">
                12小时
              </el-radio>
              <el-radio :value="24">
                24小时
              </el-radio>
              <el-radio :value="48">
                48小时
              </el-radio>
            </el-radio-group>
          </el-form-item>
        </el-tooltip>
        <el-button type="primary" native-type="submit">保存设置</el-button><span @click="ui.navigate('list')">
          <el-button type="primary">
            返回
          </el-button>
        </span><p class="setting-note">采集、快捷键和粘贴默认关闭；仅采集启用后的文本。关闭或隐藏视图继续已启用的采集。保存设置会关闭旧弹窗。</p><p class="setting-note">{{ui.state.status.listening?"后台正在监听":"后台未监听"}} · {{ui.state.status.shortcutRegistered?"快捷键已注册":"快捷键未注册"}}</p><p v-if="ui.state.status.shortcutError||ui.state.status.lastError" role="alert">{{ui.state.status.shortcutError||ui.state.status.lastError}}</p>
      </el-form>
    </el-card>
  </div>
</template>
<style scoped lang="scss">
.settings {
  flex: 1;
  padding: 16px;
  height: 100%;

  .el-form-item:last-child {
    margin-bottom: 0;
  }
}

a {
  text-decoration: none;
}

 .settings{overflow:auto;box-sizing:border-box}.setting-note{font-size:12px;line-height:1.5;margin:10px 0}</style>
