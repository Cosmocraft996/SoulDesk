(() => {
  const hostOrigin = new URLSearchParams(location.search).get('hostOrigin');
  const report = message => {
    if (hostOrigin) parent.postMessage({type:'souldesk:failed',protocol:1,message:String(message).slice(0,1000)},hostOrigin);
  };
  window.addEventListener('error', event => {
    if (event.message) { report(event.message); return; }
    const target=event.target;
    if (target instanceof HTMLImageElement) {
      const failedSource=target.src,failedSet=target.srcset;
      // 捕获阶段先于组件的onerror；下一任务再判断，给回退或显式处理留出机会。
      setTimeout(() => {
        if (!event.defaultPrevented && target.isConnected && target.src===failedSource && target.srcset===failedSet) report('组件图片加载失败');
      },0);
      return;
    }
    report('组件资源加载失败');
  }, true);
  window.addEventListener('unhandledrejection', event => report(event.reason?.message || event.reason));
})();
