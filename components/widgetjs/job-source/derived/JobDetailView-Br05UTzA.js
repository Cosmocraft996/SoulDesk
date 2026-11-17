// 固定部署派生；来源和补丁见SOURCE_FILES.json与scripts/adapt-widgetjs-job.mjs。
import{W as n,_ as c}from"./Window-li_yC5Al.js";import{a as l,aC as d,o as u,P as b,as as w,l as p,v as m,s as _,n as k,q as v,aU as f}from"./index-Bfkv8pFi.js";class g{static injectCommonCSS(r){return r.insertCSS(`
    mark{
      padding: 2px;
      background-color: #346dc8;
      color: white;
      border-radius: 4px;
    }
    ::-webkit-scrollbar {
      width: 8px;
      height: 8px;
    }

    /* 滚动条轨道 */
    ::-webkit-scrollbar-track {
      background: rgba(241, 241, 241, 0.2); /* 轨道背景色 */
      border-radius: 10px; /* 圆角 */
    }

    /* 滚动条滑块 */
    ::-webkit-scrollbar-thumb {
      background: #888; /* 滑块背景色 */
      border-radius: 10px; /* 圆角 */
    }

    /* 滚动条滑块悬停效果 */
    ::-webkit-scrollbar-thumb:hover {
      background: #555; /* 悬停时的背景色 */
    }`)}}const h=["src"],x=l({__name:"JobDetailView",setup(i){const s=f().query.url,e=d("webviewTag");return u(async()=>{var a,o,t;await b(),(a=e.value)==null||a.loadURL(s),(o=e.value)==null||o.addEventListener("dom-ready",async()=>{w.log("dom-ready"),g.injectCommonCSS(e.value)}),(t=e.value)==null||t.addEventListener("did-finish-load",()=>{})}),(a,o)=>(p(),m(n,{title:"岗位详情"},{default:_(()=>[k("webview",{ref_key:"webviewTag",ref:e,class:"webview",src:v(s),allowpopups:"",enableblinkfeatures:"PreciseMemoryInfo, CSSVariables"},null,8,h)]),_:1}))}}),S=c(x,[["__scopeId","data-v-506a5182"]]);export{S as default};
