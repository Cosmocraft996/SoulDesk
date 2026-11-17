<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{CopyOne,Delete}from'@icon-park/vue-next';import{computed}from'vue';const props=defineProps({background:{type:String,default:'rgb(99,101,255)'},icon:{type:String,required:true}}),bgOpacity20=computed(()=>'color-mix(in srgb, '+props.background+' 80%, white)');
</script>
<template>
  <div class="circle-button">
    <slot>
      <Delete v-if="icon==='delete'"/><CopyOne v-else/>
    </slot>
  </div>
</template>
<style scoped lang="scss">
@use '../original/src/assets/theme';

.circle-button {
  --bg-color: v-bind(background);
  --bg-color-opacity-20: v-bind(bgOpacity20);
  cursor: pointer;
  width: 44px;
  height: 44px;
  box-sizing: border-box;
  border-radius: 50%;
  display: flex;
  place-content: center;
  place-items: center;
  background-color: v-bind(background);
  color: white;
  font-size: 1.5em;
  transition: all 0.2s ease-in-out;
  &:active{
    transform: scale(0.9);
    box-shadow: 0 7px 20px 0 var(--bg-color-opacity-20);
  }
  &:hover {
    box-shadow: 0 7px 29px 0 var(--bg-color-opacity-20);
  }
}

</style>
