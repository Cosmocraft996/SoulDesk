import{installLibraryControl}from'../shared-library/control.js';
import{createApp}from'vue';import{connectHost}from'../../../sdk/host.js';import{restoreDocument}from'./birthday-model.js';import App from'./App.vue';import'./style.css';
const host=await connectHost();installLibraryControl(host);const initialSettings=await host.request('settings.get'),initialDocument=restoreDocument(await host.request('storage.get'));const app=createApp(App,{host,initialSettings,initialDocument});app.mount('#app');window.addEventListener('pagehide',()=>app.unmount(),{once:true});
