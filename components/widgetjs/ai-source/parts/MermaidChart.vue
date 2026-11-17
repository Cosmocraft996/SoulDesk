<!-- 固定原模板派生；来源见SOURCE_FILES.json。 -->
<script setup>
import{computed,inject,ref,watch,onMounted,onBeforeUnmount,nextTick}from'vue';import{Attention,Copy,Download,OneToOne,ZoomIn,ZoomOut}from'@icon-park/vue-next';import{mermaidSvg,savePng}from'../graphics.js';
const props=defineProps({code:String}),generator=inject('ai-generator'),{t,isDark}=generator,mermaidRef=ref(),containerRef=ref(),error=ref(''),scale=ref(1),translateX=ref(0),translateY=ref(0),isDragging=ref(false);let startX=0,startY=0,timer,sequence=0,disposed=false,renderedCode='',renderedDark=false;
const transformStyle=computed(()=>({transform:'translate('+translateX.value+'px,'+translateY.value+'px) scale('+scale.value+')',transformOrigin:'center center',transition:isDragging.value?'none':'transform .1s ease-out'}));
const zoomIn=()=>scale.value=Math.min(5,scale.value+.1),zoomOut=()=>scale.value=Math.max(.1,scale.value-.1);function fitView(){const area=containerRef.value?.getBoundingClientRect(),svg=mermaidRef.value?.querySelector('svg'),box=svg?.viewBox?.baseVal;if(!area||!box?.width||!box?.height)return;scale.value=Math.max(.1,Math.min((area.width-40)/box.width,(area.height-40)/box.height,1));translateX.value=translateY.value=0;}
async function render(){const current=++sequence,code=props.code,dark=isDark.value;if(!code?.trim()){mermaidRef.value?.replaceChildren();renderedCode='';return;}try{const svg=await mermaidSvg(code,dark?'dark':'light',()=>!disposed&&current===sequence);if(disposed||current!==sequence)return;mermaidRef.value.innerHTML=svg;renderedCode=code;renderedDark=dark;error.value='';await nextTick();fitView();}catch(e){if(!disposed&&current===sequence)error.value='图表未更新：'+e.message;}}
async function exportToPng(){try{const svg=mermaidRef.value?.querySelector('svg');if(!svg)throw Error('请先生成有效图表');if(renderedCode!==props.code||renderedDark!==isDark.value||error.value)throw Error('当前源码尚未成功渲染，请等待或修正后导出');await savePng(generator.host,svg,'mermaid-chart.png');}catch(e){error.value=e.message;}}const copy=()=>generator.copyText(props.code??'');
function onMouseDown(event){if(event.button!==0||event.target.closest('.toolbar'))return;event.preventDefault();isDragging.value=true;startX=event.clientX-translateX.value;startY=event.clientY-translateY.value;document.addEventListener('mousemove',onMouseMove);document.addEventListener('mouseup',onMouseUp);}function onMouseMove(event){if(isDragging.value){translateX.value=event.clientX-startX;translateY.value=event.clientY-startY;}}function onMouseUp(){isDragging.value=false;document.removeEventListener('mousemove',onMouseMove);document.removeEventListener('mouseup',onMouseUp);}function onWheel(event){if(event.ctrlKey){event.preventDefault();scale.value=Math.max(.1,Math.min(5,scale.value-event.deltaY*.005));}}
async function fullscreen(){try{if(document.fullscreenElement)await document.exitFullscreen();else await containerRef.value.requestFullscreen();}catch(e){error.value='无法进入全屏：'+e.message;}}onMounted(render);watch([()=>props.code,isDark],()=>{clearTimeout(timer);timer=setTimeout(render,500);});onBeforeUnmount(()=>{disposed=true;++sequence;clearTimeout(timer);onMouseUp();});
</script>
<template>
  <div
    ref="containerRef"
    class="result flex-2"
    :class="{ 'is-dragging': isDragging }"
    @mousedown="onMouseDown"
    @wheel="onWheel"
  >
    <div :style="transformStyle" class="transform-container">
      <pre ref="mermaidRef" class="mermaid-content" />
    </div>

    <div v-if="error" class="error-toast">
      <Attention theme="outline" size="16" fill="#ed4f4a" class="mr-1 mt-1" />
      <span class="error-text">{{ error }}</span>
    </div>

    <div class="toolbar"><div class="btn-icon" title="全屏预览" @click.stop="fullscreen">⛶</div>
      <div class="btn-icon" :title="t('zoomIn')" @click.stop="zoomIn">
        <ZoomIn />
      </div>
      <div class="btn-icon" :title="t('zoomOut')" @click.stop="zoomOut">
        <ZoomOut />
      </div>
      <div class="btn-icon" :title="t('fitView')" @click.stop="fitView">
        <OneToOne />
      </div>
      <div class="btn-icon" :title="t('exportPng')" @click.stop="exportToPng">
        <Download />
      </div>
      <div class="btn-icon" :title="t('copy')" @click.stop="copy">
        <Copy />
      </div>
    </div>

    <div class="controls-hint">
      {{ t('controlsHint') }}
    </div>
  </div>
</template>
<style scoped lang="scss">
.result {
  border: var(--surface-border);
  border-radius: 8px;
  position: relative;
  flex: 2;
  overflow: hidden;
  cursor: grab;
  user-select: none;

  &.is-dragging {
    cursor: grabbing;
  }

  .transform-container {
    width: 100%;
    height: 100%;
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .mermaid-content {
    font-family: inherit;
  }

  .toolbar {
    position: absolute;
    top: 12px;
    right: 12px;
    display: flex;
    gap: 8px;
    z-index: 10;
  }

  .btn-icon {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    opacity: 0.6;
    font-size: 18px;
    cursor: pointer;
    border-radius: 4px;
    border: var(--surface-border);
    background-color: var(--surface);
    transition: all 0.2s;
    color: var(--color);

    &:hover {
      opacity: 1;
      background-color: var(--surface-background);
    }
  }

  .controls-hint {
    position: absolute;
    bottom: 8px;
    right: 12px;
    font-size: 12px;
    opacity: 0.5;
    pointer-events: none;
    background-color: var(--surface);
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--color);
  }

  .error-toast {
    position: absolute;
    top: 12px;
    left: 12px;
    background: var(--surface);
    border: 1px solid #ed4f4a;
    color: var(--color);
    padding: 8px 12px;
    border-radius: 4px;
    z-index: 20;
    max-width: 80%;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    justify-items: center;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
    font-size: 12px;

    .error-text {
      flex: 1;
      word-break: break-all;
      white-space: pre-wrap;
    }
  }
}

.result{min-height:260px;min-width:0}.result:fullscreen{background:var(--background-color)}.mermaid-content{width:max-content;max-width:100%}
</style>
