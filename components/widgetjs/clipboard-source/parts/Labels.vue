<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed,inject}from'vue';import{ElMessageBox}from'element-plus';import{Plus}from'@icon-park/vue-next';const props=defineProps({modelValue:{type:Array,default:()=>[]}}),emit=defineEmits(['update:modelValue']),ui=inject('clipboard-ui'),dynamicTags=computed(()=>props.modelValue);function handleClose(tag){emit('update:modelValue',props.modelValue.filter(v=>v!==tag));}async function showInput(){try{const{value}=await ElMessageBox.prompt('请输入标签名','新建标签',{confirmButtonText:'确定',cancelButtonText:'取消',appendTo:'.clipboard',inputPattern:/^[\u4E00-\u9FA5_a-zA-Z0-9]{1,10}$/,inputErrorMessage:'最多10个字符，只能包含中文、英文、数字和下划线',inputValidator:v=>props.modelValue.includes(v)?'标签名重复':true});if(props.modelValue.length<3)emit('update:modelValue',[...props.modelValue,value]);}catch(error){if(error!=='cancel'&&error!=='close')ui.state.error=error.message??String(error);}}
</script>
<template>
  <div class="labels">
    <slot />
    <el-tag
      v-for="tag in dynamicTags"
      :key="tag"
      closable
      effect="light"
      :disable-transitions="false"
      @close="handleClose(tag)"
    >
      {{ tag }}
    </el-tag>
    <el-button v-if="dynamicTags.length < 3" size="small" type="primary" plain @click="showInput">
      <Plus />
      新标签
    </el-button>
  </div>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

.labels {
  display: flex;
  gap: 0.5rem;
}

</style>
