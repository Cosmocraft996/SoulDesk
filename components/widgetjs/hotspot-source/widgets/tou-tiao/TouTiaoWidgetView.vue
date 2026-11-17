<script lang="ts" setup>
import { ComponentFrame } from '@/native';

import { secureImage } from '@/native';

import { contentFailure } from '@/native';

import { useComponent } from '@/native'
import { ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import client from '@/client'
import HotspotItem from '@/widgets/components/HotspotItem.vue'
import HotspotBox from '@/widgets/components/HotspotBox.vue'

useComponent()
interface Res {
  data: {
    ClusterIdStr: string
    Title: string
    HotValue: string
    Image: {
      url: string
    }
    LabelUri?: {
      url: string
    }
  }[]
}

const viewList = ref<{ title: string, url: string, labelUrl?: string }[]>([])
useComponent()
useIntervalFn(() => {
  getHotList()
}, 5 * 60 * 1000, { immediate: true, immediateCallback: true })

async function getHotList() {
try {

  const res = await client.get('https://www.toutiao.com/hot-event/hot-board/?origin=toutiao_pc')
  const data = res.data as Res
  viewList.value = data.data.map((item) => {
    return {
      title: item.Title,
      url: `https://www.toutiao.com/trending/${item.ClusterIdStr}/`,
      labelUrl: item.LabelUri?.url,
    }
  })

} catch (error) { contentFailure(error); }
}
</script>

<template>
  <ComponentFrame>
    <div class="content">
      <HotspotBox class="hotspot-box">
        <template #header>
          <div class="header flex items-center gap-1">
            <img src="@/assets/icons/toutiao.png" style="width: 1rem;height: 1rem">头条热榜
          </div>
        </template>
        <template #body>
          <HotspotItem
            v-for="(item, index) in viewList"
            :key="item.url"
            :url="item.url"
            :title="item.title"
            :position="index + 1"
          >
            <template #append>
              <img v-if="item.labelUrl" :src="secureImage(item.labelUrl)" style="height: 1.2rem">
            </template>
          </HotspotItem>
        </template>
      </HotspotBox>
    </div>
  </ComponentFrame>
</template>

<style scoped>
</style>
