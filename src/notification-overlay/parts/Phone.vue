<!-- 由固定原模板派生，来源与散列见SOURCE_FILES.json。 -->
<script setup>
import{computed,ref,shallowRef,watch,onMounted,onBeforeUnmount}from'vue';
import VoiceBar from'./VoiceBar.vue';import{presets,hangupUrl,ringtoneUrl}from'../assets.js';import{createPlayback,parseLyrics,lyricAt}from'../client.js';
const props=defineProps({notification:Object,host:Object,displayed:Boolean});const emit=defineEmits(['action','error']);
const contact=computed(()=>presets[props.notification.preset]??presets.squidward),mute=computed(()=>props.notification.mute||props.host.preview),avatar=computed(()=>contact.value.avatar),title=computed(()=>props.notification.title),audio=ref(),hangup=ref(),time=ref(0),playback=shallowRef({playing:false,hanging:false,error:''});let player,started=false,mounted=false;
const desc=computed(()=>playback.value.error||(mute.value?'静音预览':lyricAt(parseLyrics(contact.value.lyric),time.value,props.notification.message))),voicePlaying=computed(()=>playback.value.playing),hangupPlaying=computed(()=>playback.value.hanging);
function ensure(){return player??=createPlayback({voice:audio.value,hangup:hangup.value},state=>props.host.request('island.audio',{id:props.notification.id,state}),value=>{playback.value=value;});}
async function answerClick(){if(mute.value)return;try{await props.host.request('island.action',{id:props.notification.id,action:'answer'});await ensure().start();}catch(error){emit('error',error.message);}}
function hangupClick(){player?.stop();player=undefined;emit('action','hangup');}
function start(){if(!mounted||started||mute.value||!props.displayed)return;started=true;void ensure().start();}
function ended(){void ensure().ended();}function finished(){void ensure().finished();}function failed(){void ensure().failed(Error('音频解码失败'));}
watch(()=>props.displayed,start);onMounted(()=>{mounted=true;start();});onBeforeUnmount(()=>player?.stop());
</script>
<template>
  <div class="phone-call">
    <audio v-if="!mute" preload="none" ref="audio" :src="contact.audio" @timeupdate="time=$event.target.currentTime" @ended="ended" @error="failed" />

    <audio v-if="!mute" preload="none" ref="hangup" :src="hangupUrl" @ended="finished" @error="failed" />
    <audio v-if="!mute" :src="ringtoneUrl" preload="none" /><img class="avatar" :src="avatar" alt="">
    <div class="content">
      <div class="title">
        {{ title }}
      </div>
      <div class="description">
        {{ desc }}
      </div>
    </div>
    <div class="actions">
      <div class="btn-hangup" role="button" aria-label="挂断" tabindex="0" @keydown.enter="hangupClick" @click="hangupClick">
        <img
          width="24"
          src="data:image/svg+xml;base64,PD94bWwgdmVyc2lvbj0iMS4wIiBzdGFuZGFsb25lPSJubyI/PjwhRE9DVFlQRSBzdmcgUFVCTElDICItLy9XM0MvL0RURCBTVkcgMS4xLy9FTiIgImh0dHA6Ly93d3cudzMub3JnL0dyYXBoaWNzL1NWRy8xLjEvRFREL3N2ZzExLmR0ZCI+PHN2ZyB0PSIxNjcxNzY5NzE5MzM2IiBjbGFzcz0iaWNvbiIgdmlld0JveD0iMCAwIDEwMjQgMTAyNCIgdmVyc2lvbj0iMS4xIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHAtaWQ9IjE2NzgiIHhtbG5zOnhsaW5rPSJodHRwOi8vd3d3LnczLm9yZy8xOTk5L3hsaW5rIiB3aWR0aD0iMjAwIiBoZWlnaHQ9IjIwMCI+PHBhdGggZD0iTTUxMiAzODRjLTY4LjQ4IDAtMTM0LjQgMTAuNjY2NjY3LTE5Ni4yNjY2NjcgMzAuNzJ2MTMyLjQ4YzAgMTYuODUzMzMzLTkuODEzMzMzIDMxLjM2LTIzLjg5MzMzMyAzOC40LTQxLjYgMjAuOTA2NjY3LTc5Ljc4NjY2NyA0Ny41NzMzMzMtMTEzLjcwNjY2NyA3OC45MzMzMzMtNy42OCA3LjQ2NjY2Ny0xOC4xMzMzMzMgMTIuMTYtMjkuODY2NjY2IDEyLjE2LTExLjczMzMzMyAwLTIyLjQtNC42OTMzMzMtMzAuMDgtMTIuNTg2NjY2TDEyLjU4NjY2NyA1NTguNTA2NjY3Yy03Ljg5MzMzMy03Ljg5MzMzMy0xMi41ODY2NjctMTguNTYtMTIuNTg2NjY3LTMwLjI5MzMzNCAwLTExLjczMzMzMyA0LjY5MzMzMy0yMi40IDEyLjU4NjY2Ny0zMC4yOTMzMzNDMTQyLjUwNjY2NyAzNzQuNCAzMTguNTA2NjY3IDI5OC42NjY2NjcgNTEyIDI5OC42NjY2NjdzMzY5LjQ5MzMzMyA3NS43MzMzMzMgNDk5LjQxMzMzMyAxOTkuMjUzMzMzYzcuODkzMzMzIDcuNjggMTIuNTg2NjY3IDE4LjU2IDEyLjU4NjY2NyAzMC4yOTMzMzMgMCAxMS43MzMzMzMtNC42OTMzMzMgMjIuNC0xMi41ODY2NjcgMzAuMDhsLTEwNS42IDEwNS42Yy03LjY4IDcuNjgtMTguMzQ2NjY3IDEyLjU4NjY2Ny0zMC4wOCAxMi41ODY2NjctMTEuNTIgMC0yMi4xODY2NjctNC42OTMzMzMtMjkuODY2NjY2LTEyLjE2YTQ5NC45NzYgNDk0Ljk3NiAwIDAgMC0xMTMuNzA2NjY3LTc4LjkzMzMzMyA0Mi45ODY2NjcgNDIuOTg2NjY3IDAgMCAxLTIzLjg5MzMzMy0zOC40di0xMzIuNDhBNjQxLjQ5MzMzMyA2NDEuNDkzMzMzIDAgMCAwIDUxMiAzODR6IiBmaWxsPSIjZmZmZmZmIiBwLWlkPSIxNjc5Ij48L3BhdGg+PC9zdmc+"
        >
      </div>
      <div v-if="!voicePlaying && !hangupPlaying" class="btn-answer" role="button" aria-label="接听" tabindex="0" @keydown.enter="answerClick" @click="answerClick">
        <img
          width="24"
          src="data:image/svg+xml;base64,PD94bWwgdmVyc2lvbj0iMS4wIiBzdGFuZGFsb25lPSJubyI/PjwhRE9DVFlQRSBzdmcgUFVCTElDICItLy9XM0MvL0RURCBTVkcgMS4xLy9FTiIgImh0dHA6Ly93d3cudzMub3JnL0dyYXBoaWNzL1NWRy8xLjEvRFREL3N2ZzExLmR0ZCI+PHN2ZyB0PSIxNjcxNzY5NzI0MTA0IiBjbGFzcz0iaWNvbiIgdmlld0JveD0iMCAwIDEwMjQgMTAyNCIgdmVyc2lvbj0iMS4xIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHAtaWQ9IjE4NTAiIHhtbG5zOnhsaW5rPSJodHRwOi8vd3d3LnczLm9yZy8xOTk5L3hsaW5rIiB3aWR0aD0iMjAwIiBoZWlnaHQ9IjIwMCI+PHBhdGggZD0iTTI4Mi40NTMzMzMgNDYwLjM3MzMzM2M2MS40NCAxMjAuNzQ2NjY3IDE2MC40MjY2NjcgMjE5LjMwNjY2NyAyODEuMTczMzM0IDI4MS4xNzMzMzRsOTMuODY2NjY2LTkzLjg2NjY2N2MxMS41Mi0xMS41MiAyOC41ODY2NjctMTUuMzYgNDMuNTItMTAuMjQgNDcuNzg2NjY3IDE1Ljc4NjY2NyA5OS40MTMzMzMgMjQuMzIgMTUyLjMyIDI0LjMyIDIzLjQ2NjY2NyAwIDQyLjY2NjY2NyAxOS4yIDQyLjY2NjY2NyA0Mi42NjY2NjdWODUzLjMzMzMzM2MwIDIzLjQ2NjY2Ny0xOS4yIDQyLjY2NjY2Ny00Mi42NjY2NjcgNDIuNjY2NjY3LTQwMC42NCAwLTcyNS4zMzMzMzMtMzI0LjY5MzMzMy03MjUuMzMzMzMzLTcyNS4zMzMzMzMgMC0yMy40NjY2NjcgMTkuMi00Mi42NjY2NjcgNDIuNjY2NjY3LTQyLjY2NjY2N2gxNDkuMzMzMzMzYzIzLjQ2NjY2NyAwIDQyLjY2NjY2NyAxOS4yIDQyLjY2NjY2NyA0Mi42NjY2NjcgMCA1My4zMzMzMzMgOC41MzMzMzMgMTA0LjUzMzMzMyAyNC4zMiAxNTIuMzIgNC42OTMzMzMgMTQuOTMzMzMzIDEuMjggMzEuNTczMzMzLTEwLjY2NjY2NyA0My41MmwtOTMuODY2NjY3IDkzLjg2NjY2NnoiIHAtaWQ9IjE4NTEiIGZpbGw9IiNmZmZmZmYiPjwvcGF0aD48L3N2Zz4="
        >
      </div>
    </div>

    <div v-if="voicePlaying" class="voice" aria-hidden="true">
      <VoiceBar />
      <VoiceBar />
      <VoiceBar />
      <VoiceBar />
      <VoiceBar />
      <VoiceBar />
      <VoiceBar />
      <VoiceBar />
      <VoiceBar background-color="#E39444" />
      <VoiceBar background-color="#E39444" />
      <VoiceBar background-color="#E39444" />
    </div>
  </div>
</template>
<style scoped lang="scss">
$containerHeight: 75px;
$childHeight: calc($containerHeight - 1.6rem);
.phone-call {
  display: flex;
  padding: 0.8rem;
  width: 100%;
  border-radius: 12px;
  align-items: center;

  .avatar {
    height: $childHeight;
    aspect-ratio: 1;
    border-radius: 50%;
  }

  .content {
    flex: 1;
    display: flex;
    align-items: flex-start;
    margin-left: 1rem;
    margin-right: 1rem;
    height: 100%;
    justify-content: space-around;
    flex-direction: column;

    .title {
      font-size: 0.8rem;
      color: rgba(255, 255, 255, 0.6);
    }

    .description {
      color: white;
      font-size: 1rem;
      text-overflow: ellipsis;
      max-width: 170px;
      white-space: nowrap;
      overflow: hidden;
    }
  }

  .voice {
    display: flex;
    align-items: center;

    .voice-bar {
      margin-right: 2px;

      &.orange {
        background-color: #E39444;
      }
    }
  }

  .actions {
    display: flex;

    .btn-hangup, .btn-answer {
      background-color: red;
      height: $childHeight;
      border-radius: 50%;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-items: center;
      justify-content: center;
      aspect-ratio: 1;

      .iconfont {
        font-size: 1.5rem;
        color: white;
      }
    }

    .btn-hangup {
      background-color: #ff453b;
      margin-right: 0.5rem;
    }

    .btn-answer {
      background-color: #32d15a;
    }

  }
}

.phone-call .content{min-width:0;margin-left:8px;margin-right:8px}.phone-call .description{max-width:140px}.phone-call .actions>div{width:32px;height:32px;flex-shrink:0}.phone-call .voice{margin-left:4px}.phone-call audio{display:none}
</style>
