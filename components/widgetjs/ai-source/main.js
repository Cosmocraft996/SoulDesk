import{createApp}from'vue';import ElementPlus from'element-plus';import'element-plus/dist/index.css';import App from'./App.vue';import'./original-theme.css';import'./markmap.css';import'./style.css';
export async function mountWidget(component,props){const app=createApp(component,props);app.use(ElementPlus);app.mount('#app');await props.host.ready();window.addEventListener('pagehide',()=>app.unmount(),{once:true});return app;}
export function mountGenerator(host,variant,view,initialContent){return mountWidget(App,{host,variant,view,initialContent});}
