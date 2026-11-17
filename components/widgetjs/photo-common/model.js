export const DEFAULT_PICTURE = 'https://images.unsplash.com/photo-1542596594-649edbc13630?q=80&w=2574&auto=format&fit=crop&ixlib=rb-4.0.3&ixid=M3wxMjA3fDB8MHxwaG90by1wYWdlfHx8fGVufDB8fHx8fA%3D%3D';

export function stickerSize(width, height, targetWidth, targetHeight) {
  if (![width,height,targetWidth,targetHeight].every(value=>Number.isFinite(value) && value>0)) throw new Error('图片尺寸无效');
  const ratio = Math.min(1,Math.max(1,targetWidth-64)/width,Math.max(1,targetHeight-64)/height);
  return [Math.max(1,Math.round(width*ratio)),Math.max(1,Math.round(height*ratio))];
}
export function validatePicture(source) {
  if (typeof source !== 'string' || !source || new TextEncoder().encode(source).length > 24*1024*1024) throw new Error('图片地址或内容无效、超出上限');
  if (/^data:image\/(?:png|jpeg|gif|webp);base64,[a-z\d+/]+=*$/i.test(source)) return source;
  const url = new URL(source);
  if (url.protocol !== 'https:' || url.username || url.password) throw new Error('图片地址需使用HTTPS；本地图片请通过文件选择');
  return source;
}
export function createCarousel(read, changed = () => {}) {
  let revision = '', generation = 0, wanted = 0, disposed = false;
  const state = {files:[],name:'',image:null,loading:false,error:''};
  function announce() { if (!disposed) changed(state); }
  async function show(index) {
    if (disposed || !state.files.length) return;
    wanted = (index % state.files.length + state.files.length) % state.files.length;
    const sequence = ++generation, name = state.files[wanted], currentRevision = revision;
    state.loading = true; state.error = ''; announce();
    try {
      const image = await read(name,currentRevision);
      if (disposed || sequence !== generation) return;
      state.image = image; state.name = name;
    } catch (error) {
      if (disposed || sequence !== generation) return;
      state.error = String(error?.message || error);
    }
    state.loading = false; announce();
  }
  return {state,
    async load(listing,random = false,rng = Math.random) {
      if (disposed) return;
      if (!listing || typeof listing.revision !== 'string' || !Array.isArray(listing.files) || listing.files.some(file=>typeof file.name !== 'string')) throw new Error('相册文件列表无效');
      ++generation; revision = listing.revision; state.files = listing.files.map(file=>file.name);
      if (random) for (let i=state.files.length-1;i>0;i--) { const j=Math.floor(rng()*(i+1)); [state.files[i],state.files[j]]=[state.files[j],state.files[i]]; }
      wanted = 0;
      if (!state.files.length) { state.name='';state.image=null;state.error='';state.loading=false;announce();return; }
      await show(0);
    },
    move: direction => show(wanted+direction),
    dispose() { disposed=true;generation++; },
  };
}

// 在脱离文档的图片节点上处理网络/解码错误，成功后再交给视图显示。
export function loadPicture(source) {
  return new Promise((resolve,reject)=>{
    const image = new Image(); image.referrerPolicy = 'no-referrer';
    const timer = setTimeout(()=>{ image.onload=null;image.onerror=null;image.src='';reject(new Error('图片加载超时，请检查地址或重新选择')); },8000);
    image.onload=()=>{ clearTimeout(timer);image.onload=null;image.onerror=null;resolve(image); };
    image.onerror=()=>{ clearTimeout(timer);image.onload=null;image.onerror=null;reject(new Error('图片无法加载或格式不受当前系统支持')); };
    image.src=source;
  });
}
