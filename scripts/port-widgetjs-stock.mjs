import{readFileSync,writeFileSync,mkdirSync}from'node:fs';
import{dirname}from'node:path';
import{spawnSync}from'node:child_process';
import{createHash}from'node:crypto';
const root='artifacts/widgetjs-upstream/stock',target='components/widgetjs/stock-source',revision='0f5157961908bb3fdec71d443117783a3496571f';
const git=spawnSync('git',['-C',root,'rev-parse','HEAD'],{encoding:'utf8'});if(git.status!==0||git.stdout.trim()!==revision)throw Error('股票固定来源不符');
const paths=['src/widgets/stock/stock-widget-view.tsx','src/widgets/stock/stock-item.tsx','src/widgets/stock/stock-config-view.tsx','src/widgets/stock/stock-widget.widget.ts','src/api/eastmoney-stock-api.ts','src/data/self-select-stock-repository.ts','src/store/use-self-select-stock-store.ts','src/store/use-stock-color-store.ts','src/components/exchange-tag.tsx','src/model/Stock.ts','src/lib/utils.ts','src/assets/main.css'];
const files=paths.map(path=>{const bytes=readFileSync(root+'/'+path),out=target+'/original/'+path;mkdirSync(dirname(out),{recursive:true});writeFileSync(out,bytes);return{path,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};});
writeFileSync(target+'/SOURCE_FILES.json',JSON.stringify({repository:'https://github.com/rtugeek/stock',revision,files},null,2)+'\n');
let row=readFileSync(root+'/src/widgets/stock/stock-item.tsx','utf8');
row=row.replace("import styled from 'styled-components'",'').replace("import { ExchangeTag } from '@/components/exchange-tag'","import { ExchangeTag } from './display'").replace("import { cn, formatMoney, formatNumber, formatPercent } from '@/lib/utils'","import { cn, formatMoney, formatNumber, formatPercent } from './display'\nimport { holdingMetrics } from './stock-model.js'\nimport { signOf } from '../metal/metal-model.js'").replace("import type { Stock } from '@/model/stock'",'type Stock = any');
const badgeStart=row.indexOf('const RatioBadge ='),badgeEnd=row.indexOf('interface MiniSparklineProps');if(badgeStart<0||badgeEnd<0)throw Error('原徽章结构变化');
row=row.slice(0,badgeStart)+'function RatioBadge({$backgroundColor,...props}: any) { return <span className="ratio-badge" style={{backgroundColor:$backgroundColor}} {...props}/> }\n\n'+row.slice(badgeEnd);
const start=row.indexOf('  const ratio = Number.parseFloat(stock.ratio)'),end=row.indexOf('\n  return (',start);if(start<0||end<0)throw Error('原指标结构变化');
row=row.slice(0,start)+`  const metrics = holdingMetrics(stock, stock.price ? stock : null)
  const ratio = Number(metrics.change?.percent || 0)
  const increase = metrics.change?.amount
  const metricValue = metricMode === 'changePercent' ? metrics.change?.percent : metricMode === 'returnRate' ? metrics.returnRate : metrics.returnAmount
  const metricText = metricValue == null ? '—' : metricMode === 'returnAmount' ? (signOf(metricValue) >= 0 ? '+' : '') + formatMoney(metricValue) : formatPercent(metricValue)
  const { color: marketColor } = getColorByValue(metrics.change?.sign || 0)
  const { color: metricColor } = metricValue == null ? {color:'#747b8c'} : getColorByValue(signOf(metricValue))
`+row.slice(end);
row=row.replace('      <defs>','      <title>示意曲线（不代表真实分时）</title>\n      <defs>').replace("exchange={stock.market || 'us'}",'exchange={String(stock.marketId)}').replace('<MiniSparkline ratio={ratio} color={marketColor} seed={stock.code} />','{stock.price ? <MiniSparkline ratio={ratio} color={marketColor} seed={String(stock.marketId)+stock.code} /> : <span>—</span>}').replace("{increase >= 0 ? '+' : ''}","{increase != null && signOf(increase) >= 0 ? '+' : ''}").replace('formatNumber(String(increase), 2)','formatNumber(increase, 2)');
writeFileSync(target+'/StockItem.tsx',row);console.log(JSON.stringify({revision,originalFiles:files.length,derived:'StockItem.tsx'}));
