<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,computed}from'vue';import{History}from'@icon-park/vue-next';import CircleButton from'./CircleButton.vue';import Labels from'./Labels.vue';const ui=inject('clipboard-ui'),selectedData=computed(()=>ui.state.draft),content=computed({get:()=>selectedData.value?.content??'',set:v=>ui.client.edit({content:v})}),labels=computed({get:()=>selectedData.value?.labels??[],set:v=>ui.client.edit({labels:v})});async function deleteData(){if(await ui.run(()=>ui.client.remove(selectedData.value)))ui.navigate('list');}
</script>
<template>
  <div v-if="selectedData" class="clipboard-detail" :inert="ui.locked.value">
    <small v-if="ui.state.stale" role="alert">记录已有变化；保存将核对打开时版本，请先保留草稿或取消重读。</small><div class="content">
      <el-input
        v-model="content" :disabled="ui.state.busy||ui.state.mutationUncertain"
        input-style="height: 100%;resize: none;border: none;outline: none;"
        type="textarea"
        :maxlength="50000"
        placeholder="请输入内容"
        show-word-limit
      />
    </div>
    <Labels v-model="labels">
      <el-tag>
        <History />
        使用{{ selectedData.useCount }}次
      </el-tag>
    </Labels>
    <div class="buttons">
      <el-popconfirm
        title="确定删除该记录？"
        width="200"
        cancel-button-text="取消"
        confirm-button-text="确定"
        @confirm="deleteData"
      >
        <template #reference>
          <CircleButton icon="delete" background="#FF6363" />
        </template>
      </el-popconfirm>
      <CircleButton icon="copy-one" title="复制已保存内容" @click="ui.submit(selectedData,false)"/><el-button :disabled="!ui.state.dirty||ui.locked.value" @click="ui.run(()=>ui.client.saveDraft())">保存</el-button><el-button @click="ui.discardDetail()">取消并返回</el-button>
    </div>
  </div>
</template>
<style scoped lang="scss">
.clipboard-detail {
  display: flex;
  flex: 1;
  flex-direction: column;
  padding: 16px;
  gap: 16px;

  .content {
    flex: 1;
  }

  .buttons {
    display: flex;
    gap: 1rem;
    width: 100%;
    place-items: center;
    place-content: center;
  }
}

 .content{min-height:80px}.clipboard-detail{min-height:0}.buttons{flex-wrap:wrap;gap:8px!important}</style>
