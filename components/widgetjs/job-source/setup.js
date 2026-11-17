// 所有派生组件使用同一个原Vue运行时；远程页面不会加载此模块。
import{inject,onMounted,onBeforeUnmount,node}from'./derived/index-Bfkv8pFi.js';
const ui=()=>{const value=inject('job-ui');if(!value)throw Error('AI直聘界面尚未连接');return value;};
export const setup=(name,props,context)=>ui().setup(name,props,context);
export const render=(name,props,context)=>ui().render(name,props,context);
export function chart(get){const resize=()=>get()?.resize();onMounted(()=>window.addEventListener('resize',resize));onBeforeUnmount(()=>{window.removeEventListener('resize',resize);get()?.dispose();});}
export const wrap=(render,extra)=>(...args)=>node('div',{class:'job-model-form'},[render(...args),extra()]);
