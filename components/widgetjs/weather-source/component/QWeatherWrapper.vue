<script setup lang="ts">
import { useWeatherContext } from '../context';
defineProps<{ errorMsg?: string }>();
const { host, model, openSettings } = useWeatherContext();
</script>
<template>
  <section class="weather-frame">
    <div v-if="host.preview || !model.state.config.configured || !model.state.config.enabled" class="weather-empty">
      <p>{{ host.preview ? '预览 · 不联网' : errorMsg || '请设置自己的和风API Host / API Key并启用联网' }}</p>
      <button v-if="!host.preview" type="button" @click="openSettings">配置天气</button>
    </div>
    <div v-else-if="!model.state.now" class="weather-empty"><p>{{ errorMsg || '正在读取天气…' }}</p><button type="button" @click="model.refresh">重试</button></div>
    <slot v-else />
  </section>
</template>
