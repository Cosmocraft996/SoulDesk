import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';import {spawnSync} from 'node:child_process';import ts from 'typescript';
const root='artifacts/widgetjs-upstream/stock',revision='0f5157961908bb3fdec71d443117783a3496571f';
const result=spawnSync('git',['-C',root,'rev-parse','HEAD'],{encoding:'utf8'});if(result.status!==0||result.stdout.trim()!==revision)throw new Error('币种目录来源提交不符');
const source=readFileSync(root+'/src/api/coin-api.ts','utf8'),start=source.indexOf('export const Coins: Coin[] = ['),end=source.indexOf('export interface IndexTicker',start);
if(start<0||end<0)throw new Error('原币种目录结构变化');
const js=ts.transpileModule(source.slice(start,end),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText;
const imported=await import('data:text/javascript;base64,'+Buffer.from(js).toString('base64'));const coins=imported.Coins;
if(coins.length!==14||coins.some(coin=>!coin.type||!coin.name||!coin.ccy||!coin.logo.startsWith('https://www.okx.com/')))throw new Error('原币种目录不完整');
mkdirSync('components/widgetjs/coin-common',{recursive:true});writeFileSync('components/widgetjs/coin-common/coins.json',JSON.stringify(coins,null,2)+'\n');console.log(JSON.stringify({coins:coins.length,revision}));
