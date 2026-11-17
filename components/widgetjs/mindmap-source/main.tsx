import {installLibraryControl} from '../shared-library/control.js';
import {createRoot} from 'react-dom/client';
import {connectHost} from '../../../sdk/host.js';
import App from './App';
import {restoreDocuments} from './document-model.js';
import {createFileAccess} from '../excalidraw-source/file-access.js';
import './base.css';
import './editor/style.css';
import './style.css';
const root=createRoot(document.getElementById('root')!);
const host=await connectHost();installLibraryControl(host);const stored=await host.request('storage.get');
try{restoreDocuments(stored);root.render(<App host={host} stored={stored} onReady={()=>void host.ready()}/>);}
catch(error){root.render(<main className="soul-mindmap-fatal"><h1>无法读取思维导图</h1><p>{String((error as Error).message)}</p><p>原数据保持不变，请保存副本后检查。</p><button disabled={host.preview} onClick={()=>void createFileAccess(host).fileSave(new Blob([JSON.stringify(stored)],{type:'application/json'}),{fileName:'思维导图-原始数据.json',extensions:['json']}).catch((error:Error)=>{document.getElementById('fatal-error')!.textContent=error.message;})}>保存原始数据副本</button><output id="fatal-error"/></main>);}
