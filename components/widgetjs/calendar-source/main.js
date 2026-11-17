import{createApp}from'vue';import{connectHost}from'../../../sdk/host.js';import App from'./App.vue';import'./style.css';
const host=await connectHost(),initialSettings=await host.request('settings.get');const app=createApp(App,{host,initialSettings});app.mount('#app');window.addEventListener('pagehide',()=>app.unmount(),{once:true});
