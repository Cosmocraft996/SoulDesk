import test from 'node:test';import assert from 'node:assert/strict';
import {readSnapshot,quoteIsOld,createFeed,coins,validateSettings,createDisplay} from './coin-model.js';
const quote={currency:'BTC',price:'12345678901234567890.125',reference:'12345678901234567890.12',timestamp:1794657600000,receivedAt:1794657600001,epoch:1};
const snapshot={provider:'OKX',source:'coin-currencies',channel:'cup-tickers-3s',connectionId:'connection',epoch:1,connection:'connected',quotes:[quote],currencyErrors:{}};
test('原14币种以网页频道身份显示，价格精确，缺基准不算零涨跌',()=>{
  assert.equal(coins.length,14);const value=readSnapshot(snapshot);assert.equal(value.quotes.get('BTC').price,quote.price);assert.equal(value.quotes.get('BTC').change.sign,1);
  assert.equal(readSnapshot({...snapshot,quotes:[{...quote,reference:null}]}).quotes.get('BTC').change,null);
  assert.throws(()=>readSnapshot({...snapshot,channel:'index-tickers'}));assert.throws(()=>readSnapshot({...snapshot,quotes:[{...quote,currency:'unknown'}]}));
  assert.throws(()=>readSnapshot({...snapshot,quotes:[quote,quote]}));assert.throws(()=>readSnapshot({...snapshot,quotes:[{...quote,price:'0'}]}));
});
test('重连、币种错误、过期及未来时间均明确视为旧数据',()=>{
  const value=readSnapshot(snapshot),q=value.quotes.get('BTC'),now=q.timestamp+3000;
  assert.equal(quoteIsOld(value,q,now),false);assert.equal(quoteIsOld({...value,epoch:2},q,now),true);
  assert.equal(quoteIsOld({...value,connection:'retrying'},q,now),true);assert.equal(quoteIsOld({...value,currencyErrors:{BTC:'源错误'}},q,now),true);
  assert.equal(quoteIsOld(value,q,q.timestamp+16000),true);assert.equal(quoteIsOld(value,q,q.timestamp-6000),true);
});
test('迟到响应和设置重置不能覆盖新连接，失败保留旧数据并报错',async()=>{
  let resolve;const model=createFeed(()=>new Promise(r=>{resolve=r;}));const old=model.refresh();model.reset();resolve({data:snapshot});await old;assert.equal(model.state.snapshot,null);
  let fail=false;const feed=createFeed(async()=>{if(fail)throw new Error('断网');return{data:snapshot};});await feed.refresh();const before=feed.state.snapshot;fail=true;await feed.refresh();assert.equal(feed.state.snapshot,before);assert.match(feed.state.error,/断网/);
});
test('刷新与代理设置保留原选项并拒绝无效参数',()=>{
  const settings={refreshMs:3000,proxyMode:'inherit',background:'#001b2d',foreground:'#ffffff',primary:'#a6adc8',border:'#41444d',radius:16};assert.equal(validateSettings(settings),settings);
  assert.throws(()=>validateSettings({...settings,refreshMs:5000}));assert.throws(()=>validateSettings({...settings,proxyMode:'http',proxyHost:'127.0.0.1',proxyPort:0}));
});
test('显示节流保留报价与时间配对，新连接或重连的新报价立即撤换旧来源',()=>{
  const display=createDisplay(),first=readSnapshot(snapshot);display.update(first,10000,1000);
  const next=readSnapshot({...snapshot,quotes:[{...quote,price:'101',timestamp:quote.timestamp+3000}]});
  assert.equal(display.update(next,10000,4000).get('BTC').price,quote.price);
  assert.equal(display.quotes.get('BTC').timestamp,quote.timestamp);
  assert.equal(display.update(next,10000,11000).get('BTC').price,'101');
  const reconnect=readSnapshot({...snapshot,epoch:2,quotes:[{...quote,price:'102',epoch:2}]});assert.equal(display.update(reconnect,10000,12000).get('BTC').price,'102');
  display.update(readSnapshot({...snapshot,connectionId:'another',quotes:[]}),10000,13000);assert.equal(display.quotes.size,0);
});

test('重开后的第一份原生快照立即显示保留报价，连接恢复前标旧且不改写时间',async()=>{
  const retained={...snapshot,connectionId:'restored',connection:'connecting',epoch:2};
  const feed=createFeed(async()=>({data:retained})),display=createDisplay();
  await feed.refresh();
  const shown=display.update(feed.state.snapshot,3000,quote.timestamp+3000).get('BTC');
  assert.equal(shown.price,quote.price);
  assert.equal(shown.timestamp,quote.timestamp);
  assert.equal(shown.receivedAt,quote.receivedAt);
  assert.equal(quoteIsOld(feed.state.snapshot,shown,quote.timestamp+3000),true);
  feed.dispose();
});
