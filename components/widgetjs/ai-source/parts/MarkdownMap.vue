<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{inject,ref,watch,onMounted,onBeforeUnmount,nextTick}from'vue';import{ZoomIn,ZoomOut,OneToOne,Download}from'@icon-park/vue-next';import{mindmapData,mountMindmap,savePng}from'../graphics.js';
const props=defineProps({markdown:{type:String,default:''}}),emit=defineEmits(['nodeClick']),generator=inject('ai-generator'),svgRef=ref(),error=ref('');let map,timer,sequence=0,disposed=false,renderedMarkdown='',drawQueue=Promise.resolve();
function update(){const current=++sequence,markdown=props.markdown;drawQueue=drawQueue.catch(()=>{}).then(async()=>{if(disposed||current!==sequence)return;if(!markdown.trim()){map?.destroy();map=undefined;svgRef.value?.replaceChildren();renderedMarkdown='';return;}try{const root=await mindmapData(markdown);if(disposed||current!==sequence)return;if(!map)map=await mountMindmap(svgRef.value,root);else await map.setData(root);if(disposed){map?.destroy();return;}if(current!==sequence)return;await map.fit();renderedMarkdown=markdown;error.value='';}catch(e){if(!disposed&&current===sequence)error.value=e.message;}});return drawQueue;}
const zoomIn=()=>map?.rescale(1.2),zoomOut=()=>map?.rescale(.8),fit=()=>map?.fit();async function exportToImage(){try{if(!map)throw Error('请先生成导图');if(renderedMarkdown!==props.markdown||error.value)throw Error('当前文本尚未成功渲染，请等待或修正后导出');const clone=svgRef.value.cloneNode(true),box=map.state.rect;clone.setAttribute('viewBox',[(box.x1??0)-20,(box.y1??0)-20,(box.x2-box.x1)+40,(box.y2-box.y1)+40].join(' '));clone.querySelector('g')?.removeAttribute('transform');await savePng(generator.host,clone,'mindmap.png',svgRef.value);}catch(e){error.value=e.message;}}
function nodeClick(event){const text=event.target.closest?.('.markmap-node')?.querySelector('foreignObject')?.textContent;if(text)emit('nodeClick',{text,x:event.clientX,y:event.clientY});}
onMounted(async()=>{await nextTick();svgRef.value.addEventListener('click',nodeClick);void update();});watch(()=>props.markdown,()=>{clearTimeout(timer);timer=setTimeout(update,400);});onBeforeUnmount(()=>{disposed=true;++sequence;clearTimeout(timer);svgRef.value?.removeEventListener('click',nodeClick);map?.destroy();});
</script>
<template>
  <div class="mind-map-wrapper">
    <svg ref="svgRef"/><p v-if="error" role="alert">{{ error }}</p>
    <div class="tools">
      <el-button-group>
        <el-button size="small" @click="zoomIn">
          <ZoomIn size="18" />
        </el-button>
        <el-button size="small" @click="zoomOut">
          <ZoomOut size="18" />
        </el-button>
        <el-button size="small" @click="fit">
          <OneToOne size="18" />
        </el-button>
        <el-button size="small" @click="exportToImage">
          <Download size="18" />
        </el-button>
      </el-button-group>
    </div>
  </div>
</template>
<style scoped lang="scss">
.mind-map-wrapper{
  position: relative;
  width: 100vw;
  overflow: hidden;
  height: 100%;
  .tools{
    position: absolute;
    bottom: 10px;
    right: 10px;
  }
  svg{
    width: 100%;
    height: 100%;
    text {
      fill: currentColor;
    }
  }
}
foreignObject{
  cursor: pointer;
  color: var(--color);
  div{
    user-select: none;
  }
}

.mind-map-wrapper{width:100%;min-height:260px;flex:1}.mind-map-wrapper svg{--markmap-text-color:var(--color);min-height:260px}.mind-map-wrapper [role=alert]{position:absolute;left:12px;bottom:12px;color:#df6159}
</style>
