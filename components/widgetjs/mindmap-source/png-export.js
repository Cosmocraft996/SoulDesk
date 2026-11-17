// X6的图像转换失败时可能不回调；超时只覆盖生成，不中断用户保存对话框。
export function createPng(graph,timeout=20000){
  return new Promise((resolve,reject)=>{
    let settled=false;const timer=setTimeout(()=>finish(new Error('PNG生成超时，请缩小导图后重试')),timeout);
    function finish(error,uri){if(settled)return;settled=true;clearTimeout(timer);if(error)reject(error);else resolve(uri);}
    try{graph.toPNG(uri=>typeof uri==='string'&&uri.startsWith('data:image/png;base64,')?finish(null,uri):finish(new Error('PNG生成结果无效')),{padding:20,backgroundColor:'transparent',quality:1});}catch(error){finish(error);}
  });
}
