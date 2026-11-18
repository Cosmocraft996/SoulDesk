<script setup>
import{provide}from'vue';import{useGenerator}from'./use-generator.js';
const props=defineProps({host:Object,variant:{type:String,default:'translate'},initialContent:String,view:{type:Object,required:true}});const generator=useGenerator(props.host,props.variant,props.initialContent);provide('ai-generator',generator);
</script>
<template><div class="ai-root" @pointerdown="generator.menu.visible=false"><component :is="view"/></div></template>
