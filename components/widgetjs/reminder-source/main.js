import{createApp,h,shallowRef,onMounted,onBeforeUnmount}from'vue';import{connectHost}from'../../../sdk/host.js';import Control from'./Control.vue';import Island from'./Island.vue';import Break from'./Break.vue';import'./style.css';
const host=await connectHost(),role=new URLSearchParams(location.search).get('role'),variant=({'sit-reminder':'sit','phone-reminder':'phone','dynamic-island':'island'})[document.body.dataset.reminderVariant]??'island';
const special=['notification-overlay','break-overlay'].includes(role);let app;
if(special){
  if(host.preview)throw Error('专用提醒层不能试加载');
  const method=role==='break-overlay'?'sit.break.peek':'island.peek',initial=await host.request(method);
  app=createApp({setup(){const snapshot=shallowRef(initial),error=shallowRef('');let timer,polling=false,disposed=false;async function poll(){if(polling||disposed)return;polling=true;try{const next=await host.request(method);if(!disposed){snapshot.value=next;error.value='';}}catch(e){if(!disposed)error.value=e.message;}finally{polling=false;}}onMounted(()=>{timer=setInterval(poll,250);});onBeforeUnmount(()=>{disposed=true;clearInterval(timer);});return()=>h('div',{class:'surface'},[h(role==='break-overlay'?Break:Island,{host,snapshot:snapshot.value,key:snapshot.value.notification?.id??snapshot.value.id}),error.value?h('small',{class:'surface-error',role:'alert'},error.value):null]);}});
}else app=createApp(Control,{host,variant});
app.mount('#app');await host.ready();window.addEventListener('pagehide',()=>app.unmount(),{once:true});
