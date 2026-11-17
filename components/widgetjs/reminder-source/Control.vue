<script setup>
import{shallowRef,computed,onMounted,onBeforeUnmount}from'vue';import{createSettings}from'./client.js';import{presets}from'./assets.js';
const props=defineProps({host:Object,variant:{type:String,default:'island'}});
const state=shallowRef();const editor=createSettings(props.host,props.variant,value=>state.value=value);state.value={...editor.state};
const locked=computed(()=>props.host.preview||state.value.loading||state.value.saving||state.value.uncertain),title=computed(()=>({island:'灵动通知',sit:'久坐提醒',phone:'来电提醒'}[props.variant]));
const weekdays=[['一',1],['二',2],['三',3],['四',4],['五',5],['六',6],['日',0]],samples=[['info','信息'],['success','成功'],['error','错误'],['warning','警告'],['reminder','久坐提醒'],['advance-countdown','提前倒计时'],['countdown','倒计时'],['call','静音来电']];
let timer,polling=false,disposed=false;const operation=shallowRef('');
function edit(key,value){try{editor.edit({...state.value.draft,[key]:value});}catch(error){operation.value=error.message;}}
function rule(index,key,value){const rules=structuredClone(state.value.draft.rules);rules[index][key]=value;edit('rules',rules);}
function day(index,value,checked){const days=state.value.draft.rules[index].workdays;rule(index,'workdays',checked?[...new Set([...days,value])]:days.filter(n=>n!==value));}
const time=minute=>String(Math.floor(minute/60)).padStart(2,'0')+':'+String(minute%60).padStart(2,'0');
function setTime(index,value){if(!/^\d{2}:\d{2}$/.test(value))return;const [hours,minutes]=value.split(':').map(Number);rule(index,'minuteOfDay',hours*60+minutes);}
async function run(method,args){if(props.host.preview)return;operation.value='';try{await props.host.request(method,args);}catch(error){operation.value=error.message;}}
async function refresh(){if(polling||disposed)return;polling=true;try{await editor.refresh();}finally{polling=false;}}
onMounted(()=>{void editor.load();timer=setInterval(refresh,3000);});onBeforeUnmount(()=>{disposed=true;clearInterval(timer);});
</script>
<template>
  <section class="control">
    <header><strong>{{ title }}</strong><span>{{ state.document.config.enabled?'后台已启用':'后台未启用' }}</span></header>
    <label class="enable"><input type="checkbox" :checked="state.draft.enabled" :disabled="locked" @change="edit('enabled',$event.target.checked)">启用{{ title }}</label>
    <p v-if="variant==='island'" class="hint">顶部显示固定提醒内容，不抢焦点。多个已启用实例共用一个通知层。</p>
    <template v-else-if="variant==='sit'">
      <p class="hint">启用后每10秒读取一次鼠标位置，仅在内存判断活动；先提醒，点击“休息一下”才进入全屏。</p>
      <label v-for="field in [['sitInterval','久坐时长（分钟）',120],['breakInterval','休息时长（分钟）',60],['mouseCheckInterval','检测阈值（分钟）',120]]" :key="field[0]" class="slider">{{ field[1] }}<output>{{ state.draft[field[0]] }}</output><input type="range" min="1" :max="field[2]" :value="state.draft[field[0]]" :disabled="locked" @input="edit(field[0],Number($event.target.value))"></label>
      <p class="hint">检测阈值：多久没使用鼠标视为离开电脑。已累计 {{ Math.floor((state.document.runtime?.usedMilliseconds??0)/60000) }} 分钟。</p>
    </template>
    <template v-else>
      <p class="hint">四种预置录音按本机时间提醒。先开启灵动通知；原录音可能含口语粗话，试听会播放声音。</p>
      <div class="table-wrap"><table><thead><tr><th>事项</th><th>状态</th><th>提醒人</th><th>提醒时间</th><th>工作日</th><th>试听</th></tr></thead><tbody><tr v-for="(item,index) in state.draft.rules" :key="item.preset">
        <td>{{ presets[item.preset].title }}</td><td><input type="checkbox" :aria-label="'启用'+presets[item.preset].title" :checked="item.enabled" :disabled="locked" @change="rule(index,'enabled',$event.target.checked)"></td>
        <td><div class="contact"><img :src="presets[item.preset].avatar" alt=""><span class="name">{{ presets[item.preset].name }}</span></div></td>
        <td><input type="time" :value="time(item.minuteOfDay)" :disabled="locked" @input="setTime(index,$event.target.value)"><small v-if="item.preset==='squidward'">提前7秒</small></td>
        <td class="days"><label v-for="[label,value] in weekdays" :key="value"><input type="checkbox" :checked="item.workdays.includes(value)" :disabled="locked" @change="day(index,value,$event.target.checked)">{{ label }}</label></td>
        <td><button :disabled="host.preview||state.saving" @click="run('phone.preview',{preset:item.preset})">试听</button></td>
      </tr></tbody></table></div>
    </template>
    <div class="save"><button :disabled="host.preview||state.saving||(!state.dirty&&!state.uncertain)" @click="editor.save()">{{ state.saving?'正在保存':state.uncertain?'重试保存':'保存设置' }}</button><button v-if="state.error&&!state.uncertain" :disabled="state.saving" @click="editor.load()">重新读取已保存设置</button><span v-if="state.dirty">有未保存修改</span></div>
    <p v-if="state.error||operation" role="alert">{{ state.error||operation }}</p>
    <p v-if="state.document.runtime?.lastError" role="alert">{{ state.document.runtime.lastError }}</p>
    <p v-if="variant!=='island'&&!state.document.runtime?.providerAvailable" class="hint">需添加并显式启用“灵动通知”来显示提醒。</p>
    <p v-if="state.document.runtime?.delivery" class="hint">最近递送：{{ ({queued:'等待显示',shown:'已显示',dismissed:'已关闭','not-shown':'未显示','audio-playing':'正在播放','audio-failed':'音频失败','audio-ended':'播放结束','display-failed':'显示失败','input-unavailable':'鼠标读取失败',expired:'已过期'})[state.document.runtime.delivery]??state.document.runtime.delivery }}</p>
    <div v-if="variant==='island'" class="samples"><button v-for="[sample,label] in samples" :key="sample" :disabled="host.preview||!state.document.config.enabled" @click="run('island.demo',{sample})">{{ label }}</button></div>
    <p v-if="host.preview" class="hint">预览保持关闭，不读取鼠标、不调度、不播放声音。</p>
  </section>
</template>
<style scoped>
.control{height:100%;overflow:auto;padding:16px;background:#17191f;color:#edf0f5;border-radius:14px;font-size:13px}header{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:14px}header strong{font-size:17px}header span,.hint,small{font-size:12px;color:#a0a8b8;line-height:1.65}.enable{display:flex;align-items:center;gap:6px;margin-bottom:10px}.slider{display:grid;grid-template-columns:1fr auto;gap:8px;margin:12px 0}.slider input{grid-column:1/-1;width:100%}input{accent-color:#74aafc}button{border:1px solid #4c5870;border-radius:7px;background:#27354d;color:#dce9ff;padding:6px 10px;cursor:pointer;font:inherit}button:disabled,input:disabled{opacity:.5;cursor:default}.save,.samples{display:flex;gap:8px;align-items:center;flex-wrap:wrap;margin-top:12px}.save span{font-size:11px;color:#c5bc8f}[role=alert]{color:#ffaaa1;white-space:pre-wrap;font-size:12px}.table-wrap{overflow:auto}table{border-collapse:collapse;width:100%;min-width:660px;font-size:12px}th,td{text-align:left;padding:8px;border-bottom:1px solid #353c4a}th{white-space:nowrap}.contact{display:flex;align-items:center}.contact img{width:28px;height:28px;border-radius:50%}.contact .name{margin-left:4px;white-space:nowrap}.days{min-width:205px}.days label{white-space:nowrap;margin-right:5px}input[type=time]{color:inherit;background:#242933;border:1px solid #46516a;padding:4px;border-radius:5px;width:92px}small{display:block}
</style>
