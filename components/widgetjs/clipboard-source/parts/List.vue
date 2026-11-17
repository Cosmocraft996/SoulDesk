<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,ref,watch,onMounted,onBeforeUnmount}from'vue';import{ElScrollbar}from'element-plus';import ClipboardItem from'./ClipboardItem.vue';import Spin from'./Spin.vue';import{handleListKey}from'../interaction.js';const ui=inject('clipboard-ui'),scrollbarRef=ref(),innerRef=ref(),spinner=ref();let observer;watch(spinner,element=>{observer?.disconnect();if(element)observer?.observe(element);},{flush:'post'});
function keyboard(event){void handleListKey(event,ui,row=>{const element=document.getElementById('item-'+row.id);scrollbarRef.value?.scrollTo({top:element?.offsetTop??0});});}
onMounted(()=>{document.addEventListener('keydown',keyboard);if(typeof IntersectionObserver!=='undefined'){observer=new IntersectionObserver(entries=>{if(entries.some(e=>e.isIntersecting))void ui.run(()=>ui.client.loadMore());});if(spinner.value)observer.observe(spinner.value);}});onBeforeUnmount(()=>{document.removeEventListener('keydown',keyboard);observer?.disconnect();});
</script>
<template>
  <aside>
    <ElScrollbar ref="scrollbarRef">
      <div ref="innerRef" class="items"><p v-if="!ui.state.items.length" class="no-more">{{ui.state.loading?"正在读取":"暂无文本记录"}}</p>
        <template v-for="item in ui.state.items" :key="item.id">
          <ClipboardItem
            :id="`item-${item.id}`"
            :favorite="item.favorite" @update:favorite="value=>ui.run(()=>ui.client.favorite(item,value))"
            :model-value="item"
            :active="ui.state.selectedId===item.id"
            @delete="ui.remove(item)" @edit="ui.openDetail(item)"
            @click="ui.submit(item)"
          />
        </template>
        <template v-if="ui.state.items.length > 0">
          <div v-if="ui.state.hasMore" ref="spinner" class="more"><Spin v-if="ui.state.loadingMore"/><el-button :disabled="ui.state.loadingMore" @click="ui.run(()=>ui.client.loadMore())">加载更多</el-button></div>
          <div v-else class="no-more">
            没有更多了
          </div>
        </template>
      </div>
    </ElScrollbar>
  </aside>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

aside {
  width: 100%;
  height: 100%;
  display: flex;
  position: relative;
  overflow: hidden;
  flex-direction: column;

  .items {
    flex: 5;
    position: relative;
  }

  .no-more {
    text-align: center;
    color: theme.$app-text-color;
    font-size: 12px;
    padding: 8px;
    margin-bottom: 8px;
  }
}

 .more{text-align:center;padding:8px}</style>
