<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{Search}from'@icon-park/vue-next';import{computed,ref,onMounted}from'vue';import{ElInput}from'element-plus';const props=defineProps({modelValue:{type:String,required:true}}),emit=defineEmits(['update:modelValue']),searchInputRef=ref(),keyword=computed({get:()=>props.modelValue,set:v=>emit('update:modelValue',v)});onMounted(()=>searchInputRef.value?.focus());
</script>
<template>
  <div class="search-box">
    <ElInput
      ref="searchInputRef"
      v-model="keyword"
      autofocus
      clearable
      style="--el-input-border-radius: 18px;"
      placeholder="输入关键词搜索"
    >
      <template #prefix>
        <Search />
      </template>
    </ElInput>
  </div>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

.search-box {
  width: 200px;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1rem;
  font-size: 1.2rem;
}

</style>
