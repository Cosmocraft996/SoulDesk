<script setup>
import{provide}from'vue';import{useGenerator}from'./use-generator.js';import Mindmap from'./views/Mindmap.vue';import Mermaid from'./views/Mermaid.vue';import Translate from'./views/Translate.vue';
const props=defineProps({host:Object,variant:{type:String,default:'translate'},initialContent:String});const generator=useGenerator(props.host,props.variant,props.initialContent);provide('ai-generator',generator);const view={mindmap:Mindmap,mermaid:Mermaid,translate:Translate}[props.variant];
</script>
<template><div class="ai-root" @pointerdown="generator.menu.visible=false"><component :is="view"/></div></template>
