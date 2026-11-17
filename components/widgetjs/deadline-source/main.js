import{createApp}from'vue';import{connectHost}from'../../../sdk/host.js';import{loadSettings}from'./deadline-model.js';import App from'./App.vue';import'./style.css';
const host=await connectHost(),initialSettings=await loadSettings(host);const app=createApp(App,{host,initialSettings});app.mount('#app');window.addEventListener('pagehide',()=>app.unmount(),{once:true});
