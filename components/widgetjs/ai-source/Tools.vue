<script setup>
import{inject,reactive,watch,computed,ref}from'vue';const generator=inject('ai-generator'),{state,client,settingsOpen,loading,uiError,notice}=generator;
const draft=reactive({apiBaseUrl:'',model:'',apiKey:'',enabled:false,noKey:false}),history=ref(''),locked=computed(()=>loading.value||state.value.busy||state.value.configUncertain),jobLabel=computed(()=>({running:'正在生成',completed:'生成完成',failed:'生成失败',cancelled:'已取消',interrupted:'上次生成已中断'})[state.value.job?.state]??'');
watch(settingsOpen,value=>{if(value&&!state.value.configUncertain){const saved=state.value.connection;Object.assign(draft,{apiBaseUrl:saved.apiBaseUrl,model:saved.model,apiKey:'',enabled:saved.enabled,noKey:saved.configured&&!saved.hasKey});}});
async function save(){if(!draft.noKey&&!draft.apiKey&&!state.value.connection.hasKey){uiError.value='请填写自己的密钥，或明确勾选无需鉴权';return;}const args={apiBaseUrl:draft.apiBaseUrl,model:draft.model,enabled:draft.enabled,...(draft.noKey?{apiKey:''}:draft.apiKey?{apiKey:draft.apiKey}:{})};try{if(await client.configure(args)){draft.apiKey='';settingsOpen.value=false;notice.value='连接设置已保存';}}catch(error){uiError.value=error.message;}}
async function clear(){if(await client.clear()){draft.apiKey='';settingsOpen.value=false;notice.value='连接已清除';}}
</script>
<template>
  <div class="generator-tools">
    <div class="tool-row"><el-button size="small" :disabled="loading" @click="settingsOpen=!settingsOpen">模型连接</el-button><span>{{ state.connection.enabled?'连接已启用':'连接未启用' }}</span><span v-if="jobLabel">{{ jobLabel }}</span><el-button v-if="state.job?.state==='running'||state.uncertain" size="small" :disabled="generator.host.preview" @click="client.cancel()">停止</el-button><el-button v-if="state.uncertain" size="small" @click="client.retry()">重试原请求</el-button><el-select v-model="history" size="small" placeholder="保存的结果" :disabled="state.job?.state==='running'" @change="generator.selectHistory"><el-option v-for="record in [...state.document.history].reverse()" :key="record.id" :value="record.id" :label="record.input.content.slice(0,24)+(record.state==='completed'?'':'（部分）')"/></el-select></div>
    <form v-if="settingsOpen" class="connection-form" @submit.prevent="save">
      <label>API Base URL<el-input v-model="draft.apiBaseUrl" :disabled="locked" placeholder="https://你的服务/v1"/></label>
      <label>模型<el-input v-model="draft.model" :disabled="locked" placeholder="填写你的模型名称"/></label>
      <label>API Key<el-input v-model="draft.apiKey" :disabled="locked||draft.noKey" type="password" autocomplete="off" :placeholder="state.connection.hasKey?'已保存，留空保留原密钥':'填写自己的密钥'"/></label>
      <label><input v-model="draft.noKey" type="checkbox" :disabled="locked">此服务无需鉴权，不发送密钥</label>
      <label><input v-model="draft.enabled" type="checkbox" :disabled="locked">启用此模型连接</label>
      <div><el-button native-type="submit" :disabled="generator.host.preview||state.busy||state.clearUncertain">{{ state.configUncertain?'重试原设置':'保存连接' }}</el-button><el-button :disabled="generator.host.preview||state.busy||state.configUncertain&&!state.clearUncertain||!state.connection.configured" @click="clear">{{ state.clearUncertain?'重试清除':'清除连接' }}</el-button></div>
      <small>请求会发送输入文本至你配置的服务；费用由服务账户承担。只在点击生成时请求，输入和结果保存在本实例本机。</small>
    </form>
    <p v-if="state.error||uiError" role="alert">{{ uiError||state.error }}</p><p v-if="state.saveError" role="alert">{{ state.saveError }}<el-button size="small" @click="generator.remember">重试保存内容</el-button></p><p v-if="notice" role="status">{{ notice }}</p><small v-if="generator.host.preview">预览不读取密钥、不联网、不生成或复制。</small><small v-if="state.job&&!state.job.resultAvailable&&state.job.state!=='running'">当前进程没有该结果，未重复发送。可读取保存的结果，或显式生成新请求。</small>
    <small v-if="state.restoreNote">{{ state.restoreNote }}</small>
  </div>
</template>
<style scoped>
.generator-tools{padding:8px;border-bottom:var(--surface-border);font-size:12px;flex-shrink:0}.tool-row{display:flex;gap:8px;align-items:center;flex-wrap:wrap}.tool-row .el-select{width:190px;margin-left:auto}.connection-form{display:flex;gap:10px;flex-wrap:wrap;margin-top:10px;padding:12px;border:var(--surface-border);border-radius:8px}.connection-form>label{display:flex;flex-direction:column;gap:4px;min-width:180px;flex:1}.connection-form>label:has(input[type=checkbox]){flex-direction:row;align-items:center}.connection-form small{width:100%;opacity:.7}.generator-tools p{margin-top:6px}[role=alert]{color:#d55c55;white-space:pre-wrap}[role=status]{color:#389567}
</style>
