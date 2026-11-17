import { useCallback, useMemo, useState } from 'react';
import { Excalidraw, serializeAsJSON } from '@excalidraw/excalidraw';
import type { ExcalidrawProps } from '@excalidraw/excalidraw/types';
import { createSceneStore, restoreScene } from './scene-model.js';
import initialData from './initialData';
import './style.css';
import '@excalidraw/excalidraw/index.css';

export default function App({host,stored,onReady}:{host:any;stored:any;onReady:()=>void}) {
  const [status,setStatus] = useState({status:'saved',error:''});
  const [message,setMessage] = useState('');
  const [store] = useState(() => createSceneStore(stored,(value:any) => host.preview ? Promise.resolve() : host.request('storage.set',value),(value:any)=>setStatus({...value})));
  const data = useMemo(() => {
    const loaded = restoreScene(stored);
    return {...initialData,...(loaded.scene || {}),libraryItems:loaded.libraryItems,scrollToContent:!loaded.scene};
  },[stored]);
  const changed = useCallback<NonNullable<ExcalidrawProps['onChange']>>((elements,appState,files) => {
    const scene = JSON.parse(serializeAsJSON(elements,appState,files,'local'));
    scene.appState = {...scene.appState,scrollX:appState.scrollX,scrollY:appState.scrollY,zoom:appState.zoom,theme:appState.theme,zenModeEnabled:appState.zenModeEnabled};
    store.updateScene(scene);
  },[store]);
  const libraryChanged = useCallback<NonNullable<ExcalidrawProps['onLibraryChange']>>(items => store.updateLibrary(items),[store]);
  const open = useCallback((value:string) => {
    if(host.preview) return;
    host.request('links.open',{url:value}).catch((error:any)=>setMessage(error.message));
  },[host]);
  const link = useCallback<NonNullable<ExcalidrawProps['onLinkOpen']>>((element,event) => {
    if(element.link && !element.link.startsWith('#')) { event.preventDefault(); open(element.link); }
  },[open]);
  return <div className="App" onClickCapture={event => {
    const anchor = (event.target as Element).closest?.('a[href]');
    const href = anchor?.getAttribute('href');
    if (href && /^https?:/i.test(href)) { event.preventDefault(); event.stopPropagation(); open(href); }
  }}>
    <div className="excalidraw-wrapper"><Excalidraw
      initialData={data} langCode="zh-CN" gridModeEnabled={true} autoFocus={false}
      onChange={changed} onLibraryChange={libraryChanged} onLinkOpen={link}
      excalidrawAPI={() => onReady()}
    /></div>
    <footer className="soul-canvas-status" role="status">
      {host.preview ? '预览 · 不保存、不打开文件' : status.status === 'error' ? '未保存：'+status.error : status.status === 'saving' ? '正在自动保存，请稍候…' : '已自动保存'}
      {status.status === 'error' && <button type="button" onClick={()=>store.retry()}>重试保存</button>}
      {message && <span>{message}<button type="button" onClick={()=>setMessage('')}>关闭提示</button></span>}
    </footer>
  </div>;
}
