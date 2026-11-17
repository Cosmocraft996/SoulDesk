<!-- SoulDesk 本机自用移植；来源及修改见 SOURCE.txt。 -->
<script lang="ts" setup>
import { useComponent, useInstanceSetting } from '@/souldesk'
import { useDateFormat, useIntervalFn, useNow } from '@vueuse/core'
import dayjs from 'dayjs'
import 'dayjs/locale/zh-cn'
import weekday from 'dayjs/plugin/weekday'
import { Lunar } from 'lunar-typescript'
import { ref } from 'vue'

dayjs.extend(weekday)
useComponent()
const time = useDateFormat(useNow(), 'HH:mm:ss')
const date = ref('')
const weekdayStr = ref('')
const lunar = ref(Lunar.fromDate(new Date()))
const font = useInstanceSetting('font', 'FiraCode')

function update() {
  date.value = dayjs().format('MM月DD日')
  weekdayStr.value = dayjs().locale('zh-cn').format('dddd')
  lunar.value = Lunar.fromDate(new Date())
}

useIntervalFn(() => {
  update()
}, 60 * 1000, { immediateCallback: true, immediate: true })
</script>

<template>
  <component-frame>
    <div class="content" :style="{ '--widget-font-family': font }">
      <div class="time">
        {{ time }}
      </div>
      <div class="text-center flex gap-2 w-full justify-center items-center">
        <div>{{ date }}</div>
        <div>{{ weekdayStr }} </div>
        <div>{{ lunar.toString().split('年')[1] }}</div>
      </div>
    </div>
  </component-frame>
</template>

<style lang='scss' scoped>

.content{
  text-align: center;
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  justify-content: center;
  color: var(--widget-color);
  .time{
    font-size: calc(4 * var(--widget-font-size));
    font-family: var(--widget-font-family, 'FiraCode'), sans-serif;
  }
}
</style>
