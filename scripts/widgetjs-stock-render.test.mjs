import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {createElement} from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {createServer} from 'vite';
import {resolve} from 'node:path';

test('完整股票保留固定原行布局，三种精确指标和缺值可渲染，曲线明确示意且不取远程旗帜',async()=>{
  const vite=await createServer({configFile:resolve('components/widgetjs/stock-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,watch:null},appType:'custom',logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]}});
  try{
    const {default:Row}=await vite.ssrLoadModule('/StockItem.tsx');
    for(const [mode,label]of[['changePercent','33.33%'],['returnRate','100.00%'],['returnAmount','0.03']]){
      const html=renderToStaticMarkup(createElement(Row,{stock:{code:'000001',marketId:1,name:'夹具证券',price:'0.2',previousClose:'0.15',holdingPrice:'0.1',holdingShares:'0.3',decimal:3},idx:0,isLast:true,metricMode:mode,getColorByValue:()=>({color:'#f82842'})}));
      assert.equal(html.includes(label),true,label);assert.equal(html.includes('示意曲线'),true);assert.equal(html.includes('SH'),true);assert.equal(html.includes('flagcdn'),false);assert.equal(html.includes('夹具证券'),true);
    }
    const empty=renderToStaticMarkup(createElement(Row,{stock:{code:'000001',marketId:0,name:'无报价',price:null,holdingPrice:'',holdingShares:''},idx:0,isLast:true,metricMode:'returnAmount',getColorByValue:()=>({color:'#aaa'})}));
    assert.equal(empty.includes('—'),true);assert.equal(empty.includes('+0.00'),false);
    const source=JSON.parse(readFileSync('components/widgetjs/stock-source/SOURCE_FILES.json'));assert.equal(source.revision,'0f5157961908bb3fdec71d443117783a3496571f');
    for(const file of source.files)assert.equal(createHash('sha256').update(readFileSync('components/widgetjs/stock-source/original/'+file.path)).digest('hex'),file.sha256,file.path);
  }finally{await vite.close();}
});

test('完整股票实际根视图保留列表、持仓表单和来源提示，空预览不联网',async()=>{
  const vite=await createServer({configFile:resolve('components/widgetjs/stock-source/vite.config.mjs'),server:{middlewareMode:true,hmr:false,watch:null},appType:'custom',logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]}});
  try{
    const {default:App}=await vite.ssrLoadModule('/App.tsx');
    const manifest=JSON.parse(readFileSync('components/widgetjs/stock/manifest.json'));const initialSettings=Object.fromEntries(Object.entries(manifest.settingsSchema.properties).map(([key,value])=>[key,value.default]));let calls=0;
    const host={preview:true,request(){calls++;throw Error('预览不请求');}};
    const html=renderToStaticMarkup(createElement(App,{host,initialSettings,initialPortfolio:{version:1,stocks:[{code:'000001',marketId:0,name:'夹具股票',holdingPrice:'0.1',holdingShares:'0.3'}]}}));
    for(const label of ['夹具股票','持有份额','持仓价','预览','不关联交易账户','曲线仅示意','自选股票','未计税费'])assert.equal(html.includes(label),true,label);
    assert.equal(html.includes('undefined'),false);assert.equal(calls,0);
  }finally{await vite.close();}
});
