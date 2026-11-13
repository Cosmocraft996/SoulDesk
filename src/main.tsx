import { createRoot } from 'react-dom/client';
import { App } from './App';
import { DesktopProbe } from './DesktopProbe';
import { WidgetFrame } from './WidgetFrame';
import './style.css';

const probeKind = new URLSearchParams(location.search).get('desktopProbe');
const probe = probeKind !== null && ['clock', 'card', 'panel', 'trial'].includes(probeKind);
if (probe) document.documentElement.dataset.desktopProbe = probeKind;
createRoot(document.getElementById('root')!).render(probeKind === 'trial' ? <WidgetFrame probe instance={new URLSearchParams(location.search).get('instance') ?? ''} /> : probe ? <DesktopProbe kind={probeKind} /> : <App />);
