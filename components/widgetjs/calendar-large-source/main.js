import{installLibraryControl}from'../shared-library/control.js';
import{createApp}from'vue';import{connectHost}from'../../../sdk/host.js';import App from'./App.vue';import'./style.css';
const host=await connectHost();installLibraryControl(host,{nativeKind:'todo'});const initialSettings=await host.request('settings.get'),initialSnapshot=host.preview?{version:1,revision:0,todos:[],reminders:[],sampledAt:Date.now()}:await host.request('todo.get');const app=createApp(App,{host,initialSettings,initialSnapshot});app.mount('#app');window.addEventListener('pagehide',()=>app.unmount(),{once:true});
