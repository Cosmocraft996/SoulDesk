import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const source=readFileSync(new URL('./runtime.js',import.meta.url),'utf8');
function runtime(){
  const callbacks={},tasks=[],messages=[];
  class Image {constructor(src){this.src=src;this.isConnected=true;}}
  runInNewContext(source,{URLSearchParams,location:{search:'?hostOrigin=https%3A%2F%2Fsouldesk.example'},parent:{postMessage:(data,origin)=>messages.push({data,origin})},window:{addEventListener:(type,callback)=>{callbacks[type]=callback;}},HTMLImageElement:Image,setTimeout:callback=>tasks.push(callback)});
  return {callbacks,messages,Image,flush(){while(tasks.length)tasks.shift()();}};
}
test('图片回退后不误报失败，最终回退图仍失败时报告',()=>{
  const env=runtime(),image=new env.Image('https://example.org/logo.png');
  env.callbacks.error({target:image});
  assert.equal(env.messages.length,0);
  image.src='http://widget.localhost/local.png';env.flush();assert.equal(env.messages.length,0);
  env.callbacks.error({target:image});env.flush();assert.equal(env.messages.length,1);
  assert.equal(env.messages[0].data.type,'souldesk:failed');assert.equal(env.messages[0].origin,'https://souldesk.example');
});
test('组件已处理或已移除的图片错误不会覆盖当前组件状态',()=>{
  const env=runtime(),event={target:new env.Image('https://example.org/a.png'),defaultPrevented:false};
  env.callbacks.error(event);event.defaultPrevented=true;env.flush();assert.equal(env.messages.length,0);
  const image=new env.Image('https://example.org/b.png');env.callbacks.error({target:image});image.isConnected=false;env.flush();assert.equal(env.messages.length,0);
});
test('脚本异常、拒绝未处理及关键资源失败继续报告',()=>{
  const env=runtime();env.callbacks.error({message:'脚本异常',target:{}});env.callbacks.unhandledrejection({reason:new Error('请求未处理')});env.callbacks.error({target:{tagName:'SCRIPT'}});env.flush();
  assert.equal(env.messages.length,3);assert.equal(env.messages[0].data.message,'脚本异常');assert.equal(env.messages[1].data.message,'请求未处理');
});
