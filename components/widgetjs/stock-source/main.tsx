import{createRoot}from'react-dom/client';
import{connectHost}from'../../../sdk/host.js';
import App from'./App';
import'./style.css';
const host=await connectHost(),initialSettings=await host.request('settings.get'),initialPortfolio=await host.request('storage.get');
const root=createRoot(document.getElementById('app')!);root.render(<App host={host} initialSettings={initialSettings} initialPortfolio={initialPortfolio}/>);
window.addEventListener('pagehide',()=>root.unmount(),{once:true});
