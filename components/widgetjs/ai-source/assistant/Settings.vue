<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed,inject,reactive,ref,watch}from'vue';const ui=inject('assistant-ui'),t=ui.t,copy=v=>JSON.parse(JSON.stringify(v)),draft=reactive(copy(ui.state.value.config)),baseline=ref(ui.state.value.revision),mode=ref(draft.proxy.mode),proxy=reactive(draft.proxy.mode==='manual'?copy(draft.proxy):{protocol:'http',host:'127.0.0.1',port:7890}),saved=ref(false),conflict=computed(()=>baseline.value!==ui.state.value.revision);watch([draft,mode,proxy],()=>saved.value=false,{deep:true});async function save(rebase=false){if(rebase)baseline.value=ui.state.value.revision;const value={...copy(draft),proxy:mode.value==='system'?{mode:'system'}:{mode:'manual',protocol:proxy.protocol,host:proxy.host,port:proxy.port}};if(await ui.client.save(value,baseline.value)){baseline.value=ui.state.value.revision;saved.value=true;}}async function retry(){if(await ui.client.retrySave()){baseline.value=ui.state.value.revision;saved.value=true;}}async function latest(){if(await ui.client.refresh()){Object.assign(draft,copy(ui.state.value.config));baseline.value=ui.state.value.revision;mode.value=draft.proxy.mode;if(mode.value==='manual')Object.assign(proxy,copy(draft.proxy));saved.value=false;}}
</script>
<template>
  <section class="assistant-settings"><header><h3>{{t('configTitle')}}</h3><el-button @click="ui.settings(false)" :disabled="ui.state.value.busy||ui.state.value.uncertain">返回</el-button></header>
    <div class="settings-body">
      <el-form :label-width="110" label-position="left" @submit.prevent="save()" :disabled="ui.locked.value"><el-form-item label="后台入口"><el-switch v-model="draft.backgroundEnabled"/></el-form-item><el-form-item label="置顶"><el-switch v-model="draft.alwaysTop"/></el-form-item>
        <el-form-item :label="t('shortcut')">
          <el-input v-model="draft.shortcut" maxlength="80" placeholder="Super+Alt+C"/>
        </el-form-item>
        <el-form-item :label="t('position')">
          <el-radio-group v-model="draft.position">
            <el-radio value="left">
              {{ t('left') }}
            </el-radio>
            <el-radio value="right">
              {{ t('right') }}
            </el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item label="代理"><el-radio-group v-model="mode"><el-radio value="system">跟随系统</el-radio><el-radio value="manual">手动配置</el-radio></el-radio-group></el-form-item><template v-if="mode==='manual'"><el-form-item label="协议"><el-select v-model="proxy.protocol"><el-option v-for="p in ['http','https','socks4','socks5']" :key="p" :label="p.toUpperCase()" :value="p"/></el-select></el-form-item><el-form-item label="代理主机"><el-input v-model="proxy.host" maxlength="253"/></el-form-item><el-form-item label="端口"><el-input-number v-model="proxy.port" :min="1" :max="65535"/></el-form-item></template>
        <el-form-item :label="t('windowSize')">
          <el-radio-group v-model="draft.widthRatio">
            <el-radio :value="0.2">
              {{ t('percent20') }}
            </el-radio>
            <el-radio :value="0.3">
              {{ t('percent30') }}
            </el-radio>
            <el-radio :value="0.4">
              {{ t('percent40') }}
            </el-radio>
            <el-radio :value="0.5">
              {{ t('percent50') }}
            </el-radio>
          </el-radio-group>
        </el-form-item>
      <el-button native-type="submit" type="primary">保存设置</el-button></el-form><p v-if="ui.error.value" role="alert">{{ui.error.value}}</p><p v-if="saved&&!ui.error.value">已保存</p><p v-if="conflict&&!ui.state.value.uncertain">其他窗口已更新设置，当前草稿保留。<el-button @click="latest">读取最新设置</el-button><el-button @click="save(true)">按最新版本保存草稿</el-button></p><el-button v-if="ui.state.value.uncertain" :disabled="ui.state.value.busy" @click="retry">重试原保存请求</el-button><p class="setting-note">后台入口和置顶默认关闭。启用后台入口后可用快捷键或托盘打开，收起时保留网页；关闭后台入口后收起会释放网页。各平台由你手动登录，切换代理会重新连接并保留登录资料。</p><p class="setting-note">{{ui.state.value.shortcutRegistered?'快捷键已注册':'快捷键未注册'}} {{ui.state.value.shortcutError}}</p>
    </div>
  </section>
</template>
<style lang="scss">
.title-bar{
  app-region: drag;
}
.actions{
  app-region: no-drag;
  cursor: pointer;
}

 .assistant-settings{box-sizing:border-box;padding:16px;color:var(--el-text-color-primary,#303133)}.assistant-settings header{display:flex;justify-content:space-between;align-items:center;margin-bottom:16px}.assistant-settings h3{margin:0;font-size:16px}.setting-note{font-size:12px;line-height:1.6}.assistant-settings [role=alert]{color:var(--el-color-danger,#f56c6c);line-height:1.6}</style>
