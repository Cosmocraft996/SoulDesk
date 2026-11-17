<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed}from'vue';import dayjs from'dayjs';import{Delete,Edit}from'@icon-park/vue-next';import FavoriteButton from'./FavoriteButton.vue';const props=defineProps({modelValue:{type:Object,required:true},active:Boolean,favorite:Boolean}),emits=defineEmits(['click','delete','edit','update:favorite']),time=computed(()=>dayjs(props.modelValue.latestUseTime).format('MM-DD HH:mm')),favoriteModel=computed({get:()=>props.favorite,set:v=>emits('update:favorite',v)});
</script>
<template>
  <div class="clipboard-item" :class="{ active }" @click.self="emits('click')">
    <div class="info" @click="emits('click')">
      <div class="content">
        {{ modelValue.content.length>120?modelValue.content.slice(0,120)+"...":modelValue.content }}
      </div>
      <div class="desc">
        <span>最近使用：{{ time }}</span>
        <span>使用次数：{{ modelValue.useCount }}</span>
      </div>
    </div>
    <div class="actions">
      <FavoriteButton v-model="favoriteModel" @click.stop/>
      <Edit title="查看和编辑" @click.stop="emits('edit')"/><Delete title="删除" @click.stop="emits('delete')"/>
    </div>
  </div>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

$active-color: theme.$app-secondary-color;

.clipboard-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 4px 8px;
  cursor: pointer;
  margin: 8px 8px;
  border-radius: 0.5rem;
  position: relative;
  background-color: theme.$app-card-color;
  transition: all 0.2s ease-in-out;
  border: 4px solid rgba(255, 255, 255, 0.5);

  &:hover {
    border: 4px solid rgba(255, 168, 125, 0.32);
  }

  &.active {
    border: 4px solid $active-color;
  }

  .actions {
    display: flex;
    gap: 12px;
    align-items: center;

    .i-icon {
      font-size: 1.2em;
    }
  }

  .info {
    flex: 1;
    display: flex;
    flex-direction: column;
    user-select: none;
    gap: 0.5rem;
    width: 70%;
    line-height: 1.2rem;

    .content {
      font-size: 1rem;
      font-weight: bold;
      white-space: pre-wrap;
      word-wrap: break-word;
    }

    .desc {
      font-size: 14px;
      color: #999;
      display: flex;
      gap: 1rem;
    }
  }
}

</style>
