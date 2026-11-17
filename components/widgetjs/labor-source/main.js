import{createApp}from'vue';import{connectHost}from'../../../sdk/host.js';import App from'./App.vue';import'./style.css';
const host=await connectHost(),settings=await host.request('settings.get');const app=createApp(App,{host,initialSettings:settings});app.mount('#app');window.addEventListener('pagehide',()=>app.unmount(),{once:true});
