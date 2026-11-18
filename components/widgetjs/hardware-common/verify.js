// 仅供显式隔离验收调用；不模拟硬件数值，不改变正常采样和显示。
export async function verifyHardware({host,kind,model,getSettings,expectedView}) {
  if(host.preview||new URLSearchParams(location.search).get('verify')!=='widgetjs')return;
  const $=id=>document.getElementById(id),storage=await host.request('storage.get'),prior=storage.verification||{},background='#'+host.instanceId.replaceAll('-','').slice(0,6);
  async function until(test){for(let n=0;n<160;n++){if(test())return;await new Promise(resolve=>setTimeout(resolve,25));}throw Error('硬件组件验收条件等待超时');}
  async function sample(){await until(()=>!model.state.loading);await model.refresh();if(model.state.snapshot?.cpu?.percent===null&&/预热/.test(model.state.snapshot.cpu.error||'')){await new Promise(resolve=>setTimeout(resolve,3200));await model.refresh();}return model.state.snapshot;}
  async function submit(values){$('settings').click();for(const[key,value]of Object.entries(values))$(key).value=String(value);$('form').requestSubmit();await until(()=>!$('config').open);}
  let invalidSaveRejected=prior.invalidSaveRejected===true,actionsVerified=prior.actionsVerified===true;
  try{
    let raw=await sample();if(!raw)throw Error(model.state.error||'缺少原生读数');
    if(!prior.uiVerified){
      $('settings').click();const before=await host.request('settings.get'),form=$('form');form.noValidate=true;$('radius').value='999';form.requestSubmit();await until(()=>$('save-error').textContent.length>0);
      invalidSaveRejected=$('config').open&&JSON.stringify(before)===JSON.stringify(await host.request('settings.get'));$('close').click();form.noValidate=false;
      if(kind==='energy-label'){
        await submit({productor:'验'+host.instanceId.slice(0,8),spec:'本机'+host.instanceId.slice(-6),fontSize:16,radius:12});raw=await sample();const expected=expectedView(raw);actionsVerified=$('cpu').textContent===expected.cpuText&&$('memory').textContent===expected.memoryText&&$('level').textContent===(expected.level===null?'—':String(expected.level));
      }else{
        await submit({fontSize:16,background,color:'#ffffff',divider:'#ffffff',radius:12});raw=await sample();const expected=expectedView(raw);actionsVerified=['cpu','memory','gpu'].every(key=>$(key+'-value').textContent===expected[key].label&&$(key+'-subtitle').textContent===expected[key].subtitle)&&$('download').textContent==='↓ '+expected.network.download&&$('upload').textContent==='↑ '+expected.network.upload&&$('received').textContent===expected.network.received&&$('sent').textContent===expected.network.sent;
        const canvas=$('cpu-canvas');await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));storage.canvasRendered=[...canvas.getContext('2d').getImageData(0,0,100,100).data].some((value,index)=>index%4===3&&value>0);actionsVerified&&=storage.canvasRendered;
      }
    }
    raw=await sample();const settings=getSettings(),images=[...document.querySelectorAll('main img')];await Promise.all(images.map(image=>image.decode()));const imagesLoaded=images.every(image=>image.naturalWidth>0),stylesLoaded=getComputedStyle(document.querySelector('main')).borderRadius==='12px';
    const configurationSaved=kind==='energy-label'?settings.productor==='验'+host.instanceId.slice(0,8)&&settings.spec==='本机'+host.instanceId.slice(-6)&&settings.fontSize===16&&settings.radius===12:settings.background===background&&settings.color==='#ffffff'&&settings.radius===12;
    const metricsHonest=['cpu','memory','gpu'].every(key=>raw[key].percent===null?typeof raw[key].error==='string'&&raw[key].error.length>0:Number.isFinite(raw[key].percent)&&raw[key].percent>=0&&raw[key].percent<=100);
    await host.request('storage.set',{...storage,verification:{...prior,runtimeLoads:(prior.runtimeLoads||0)+1,invalidSaveRejected,actionsVerified,imagesLoaded,stylesLoaded,configurationSaved,metricsHonest,rawSnapshot:raw,statusText:$('status').textContent,savedLabel:kind==='energy-label'?settings.productor:undefined,uiVerified:invalidSaveRejected&&actionsVerified&&imagesLoaded&&stylesLoaded&&configurationSaved&&metricsHonest}});
  }catch(error){await host.request('storage.set',{...storage,verification:{...prior,failure:String(error?.message||error)}});}
}
