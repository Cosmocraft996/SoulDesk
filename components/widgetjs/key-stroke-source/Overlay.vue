<script setup>
import {computed} from 'vue';
const props=defineProps({snapshot:{type:Object,required:true}});
const modifierKeys=computed(()=>props.snapshot.modifierKeys||[]),normalKey=computed(()=>props.snapshot.normalKey),capsLock=computed(()=>props.snapshot.capsLock),isCapsLockOn=computed(()=>props.snapshot.isCapsLockOn);
</script>

<template>
  <div class="container">
    <div
        
        class="keyboard" :class="{'is-visible': snapshot.visible}"
    >
      <div class="keyboard-inner">
        <template v-for="modifierKey in modifierKeys" :key="modifierKey.keyCode">
          <div class="key">
            <div class="key-content" :class="{ active: !modifierKey.isKeyUp }">
              {{ modifierKey.name }}
            </div>
          </div>
        </template>

        <div v-if="normalKey" class="key">
          <div class="key-content" :class="{ active: !normalKey.isKeyUp }">
            {{ normalKey?.name }}
          </div>
        </div>
        <div v-if="capsLock" class="key">
          <div class="key-content caps-lock" :class="{ active: !capsLock.isKeyUp, on: isCapsLockOn }">
            Caps
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped lang="scss">
.container {
  display: flex;
  align-items: start;
  justify-content: center;
  font-family: system-ui;
  width: 100vw;
  height: 100vh;
  padding: 6px;
}

$radius: 20px;
$key-height: 60px;
.keyboard {
  --radius: 20px;
  --red: rgb(255, 100, 100);
  --green: rgb(100, 200, 100);
  --blue: rgb(100, 100, 255);
  padding: 8px;
  background-color: #111;
  border-radius: $radius;
  box-shadow: 0 4px #0b0b0b, -2px 5px 4px #0a0a0a;
  box-sizing: border-box;
  display: flex;
  margin: 6px;
  width: fit-content;

  .keyboard-inner {
    display: flex;
    gap: 0.5rem;
    padding: 0.5rem;
    overflow: hidden;
    background-color: #222;
    border-radius: calc(var(--radius) * 0.9);
    box-shadow: 0 0.2rem rgba(0, 0, 0, 1) inset;
    justify-items: center;

    .key {
      width: $key-height;
      height: $key-height;
      padding: 0.3rem 0.2rem 0.3rem;
      top: 0.3rem;
      position: relative;
      overflow: hidden;
      border-radius: calc(var(--radius) * 0.5);
      transform: translate3d(0, 0, 0); // safari fix lol
      animation: shadow 5s -0.5s linear infinite;

      --delay: 0;

      .key-content {
        animation: color 5s calc(-0.5s + var(--delay)) linear infinite;
      }

      @keyframes shadow {
        0% {
          box-shadow: 0 0 5rem -2rem var(--red);
        }
        33% {
          box-shadow: 0 0 5rem -2rem var(--blue);
        }
        66% {
          box-shadow: 0 0 5rem -2rem var(--green);
        }
        100% {
          box-shadow: 0 0 5rem -2rem var(--red);
        }
      }

      &:before {
        content: '';
        width: $key-height * 2;
        height: $key-height * 2;
        position: absolute;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%);
        background: conic-gradient(var(--red), var(--green), var(--blue), var(--red));
        animation: spin 3s 0s linear infinite;

        @keyframes spin {
          from {
            transform: translate(-50%, -50%) rotate(0deg);
          }
          to {
            transform: translate(-50%, -50%) rotate(360deg);
          }
        }
      }

      .key-content {
        width: 100%;
        height: 100%;
        background-color: #111;
        position: relative;
        border-radius: calc(var(--radius) * 0.4);
        font-size: 1.1rem;
        top: -0.1rem;

        &.caps-lock {
          &::before {
            width: 6px;
            height: 6px;
            top: 6px;
            position: absolute;
            content: '';
            right: 6px;
            border-radius: 50%;
            background-color: #a2a2a2;
            box-shadow: 0 0 2px 2px rgba(235, 235, 235, 0.46);
          }
          &.on{
            &::before {
              background-color: #85f120;
              box-shadow: 0 0 2px 2px rgba(133, 241, 32, 0.48);
            }
          }
        }
        display: flex;
        align-items: center;
        justify-content: center;
        color: #fff;
        z-index: 1;
        cursor: pointer;
        user-select: none;
        box-shadow: 0 0.2rem rgba(0, 0, 0, 0.6);
        transition: top 0.1s ease-in-out, box-shadow 0.1s ease-in-out, background-color 0.1s ease-in-out;

        &.active,
        &:active {
          top: 0;
          box-shadow: 0 0.1rem rgba(0, 0, 0, 0.6);
          background-color: rgba(#111, 0.5);
          //animation: wiggle 0.5s ease-in-out infinite;
        }

        @keyframes wiggle {
          0% {
            transform: translate(0, 0) rotate(0);
          }
          10% {
            transform: translate(-1px, -2px);
          }
          20% {
            transform: translate(2px, 2px);
          }
          30% {
            transform: translate(-1px, -2px);
          }
          40% {
            transform: translate(2px, 1px);
          }
          50% {
            transform: translate(-1px, -2px);
          }
          60% {
            transform: translate(1px, 2px);
          }
          70% {
            transform: translate(-2px, -2px);
          }
          80% {
            transform: translate(1px, 2px);
          }
          90% {
            transform: translate(-2px, -1px);
          }
          100% {
            transform: translate(0, 0);
          }
        }

        @keyframes color {
          0% {
            color: var(--red);
          }
          33% {
            color: var(--blue);
          }
          66% {
            color: var(--green);
          }
          100% {
            color: var(--red);
          }
        }
      }
    }
  }
}
.keyboard {opacity:0;transform:translateY(150px);transition:transform .3s cubic-bezier(.2,.8,.2,1),opacity .3s;pointer-events:none;}
.keyboard.is-visible {opacity:1;transform:translateY(0);}
</style>
