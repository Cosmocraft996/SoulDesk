// 工具内容使用固定后端命名空间，导图/翻译不能覆盖父实例或互相的存储。
export function toolHost(host){return{...host,request(method,args=null){if(method==='storage.get')return host.request('ai.document.get');if(method==='storage.set')return host.request('ai.document.set',{document:args});return host.request(method,args);}};}
