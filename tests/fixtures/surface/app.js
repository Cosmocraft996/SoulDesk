import { connectHost } from './host.js';
const host=await connectHost(),settings=await host.request('settings.get');
const key='message' in settings?'message':'title',input=document.getElementById('input');
function show(value){document.getElementById('message').textContent=input.value=value[key]||'';}show(settings);
let saving=Promise.resolve();
document.getElementById('save').addEventListener('click',()=>{saving=(async()=>{try{const next=await host.request('settings.update',{...settings,[key]:input.value});show(next);document.getElementById('status').textContent='已保存';}catch(error){document.getElementById('status').textContent='保存失败';throw error;}})();void saving.catch(()=>{});});
await host.ready();
if(new URLSearchParams(location.search).has('verify')){input.value='实例-'+host.instanceId.slice(0,8);document.getElementById('save').click();await saving;const saved=await host.request('settings.get');await host.request('storage.set',{moduleExecuted:true,buttonSave:saved[key]===input.value,savedMessage:saved[key],standardsMode:document.compatMode,origin:location.origin});}
