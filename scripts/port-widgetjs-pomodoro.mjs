// 静态提取原图标和CSS；不执行原登录、统计或WidgetJS入口。
import{readFileSync,writeFileSync,readdirSync}from'node:fs';import{createHash}from'node:crypto';import ts from'typescript';
import{inflateSync}from'node:zlib';
const root='components/widgetjs/pomodoro-source/',icons={};
if(createHash('sha256').update(readFileSync(root+'original/PomodoroWidgetView-eY2LxrFN.js')).digest('hex')!=='cf1794c3877b13b270695db15750807340ef08de848c42cc36395cad6d5a5c8c'||readFileSync(root+'original/main-window-constants-excerpt.js','utf8')!=='static SIZE_POMODORO_WINDOW=150')throw Error('固定番茄视图或尺寸摘录变化');
function literal(n){if(ts.isStringLiteral(n))return n.text;if(ts.isNumericLiteral(n))return Number(n.text);if(ts.isArrayLiteralExpression(n))return n.elements.map(literal);if(ts.isObjectLiteralExpression(n))return Object.fromEntries(n.properties.map(p=>{if(!ts.isPropertyAssignment(p))throw Error('非静态图标');return[p.name.getText().replace(/^['"]|['"]$/g,''),literal(p.initializer)];}));throw Error('非静态节点');}
for(const name of ['PomodoroWidgetView-eY2LxrFN.js','check-Bcr9P__C.js','chevron-right-Db1pJddY.js','plus-BN9Q8hJ9.js']){const ast=ts.createSourceFile(name,readFileSync(root+'original/'+name,'utf8'),ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);function visit(n){if(ts.isObjectLiteralExpression(n)&&n.properties.some(p=>ts.isPropertyAssignment(p)&&p.name.getText()==='node')){const v=literal(n);if(v.name&&v.node)icons[v.name]=v.node;}ts.forEachChild(n,visit);}visit(ast);}
for(const name of ['grip','pause','play','check','chevron-right','plus'])if(!icons[name])throw Error('缺少原图标：'+name);
writeFileSync(root+'icons.js','// 原固定部署的静态图标。\nexport const icons='+JSON.stringify(icons)+';\n');
writeFileSync(root+'original-style.css',readFileSync(root+'original/PomodoroWidgetView-M92tUwNB.css','utf8').replaceAll(/\[data-v-[a-f0-9]+\]/g,''));
// 原ICO只有一个32×32 RGBA PNG帧；静态解码为Tauri托盘所需字节，不增加图像依赖。
const ico=readFileSync(root+'original/pomodoro.ico');
if(createHash('sha256').update(ico).digest('hex')!=='6505c41f648ac13a56cad0f6b6731509910ec1034264b1a6d27dae4f520f462c')throw Error('固定托盘图标字节变化');
if(ico.readUInt32LE(0)!==65536||ico.readUInt16LE(4)!==1||ico[6]!==32||ico[7]!==32)throw Error('原托盘ICO目录变化');
const offset=ico.readUInt32LE(18),length=ico.readUInt32LE(14),png=ico.subarray(offset,offset+length);
if(offset+length!==ico.length||!png.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])))throw Error('原托盘不是固定PNG帧');
let position=8,header=false,ended=false;const chunks=[];
while(position+12<=png.length){const size=png.readUInt32BE(position),type=png.toString('ascii',position+4,position+8);if(position+12+size>png.length)throw Error('托盘PNG块越界');const data=png.subarray(position+8,position+8+size);if(type==='IHDR'){if(header||size!==13||data.readUInt32BE(0)!==32||data.readUInt32BE(4)!==32||data[8]!==8||data[9]!==6||data[10]||data[11]||data[12])throw Error('托盘RGBA尺寸或编码变化');header=true;}if(type==='IDAT')chunks.push(data);position+=12+size;if(type==='IEND'){ended=true;break;}}
if(!header||!ended||position!==png.length)throw Error('原托盘PNG不完整');
const filtered=inflateSync(Buffer.concat(chunks),{maxOutputLength:32*(32*4+1)}),rgba=Buffer.alloc(32*32*4);if(filtered.length!==32*(32*4+1))throw Error('托盘PNG扫描线不完整');
function paeth(a,b,c){const p=a+b-c,pa=Math.abs(p-a),pb=Math.abs(p-b),pc=Math.abs(p-c);return pa<=pb&&pa<=pc?a:pb<=pc?b:c;}
for(let y=0;y<32;y++){const filter=filtered[y*129];if(filter>4)throw Error('托盘PNG滤波类型无效');for(let x=0;x<128;x++){const index=y*128+x,left=x>=4?rgba[index-4]:0,up=y?rgba[index-128]:0,upperLeft=y&&x>=4?rgba[index-132]:0;const predictor=filter===0?0:filter===1?left:filter===2?up:filter===3?Math.floor((left+up)/2):paeth(left,up,upperLeft);rgba[index]=(filtered[y*129+x+1]+predictor)&255;}}
if(createHash('sha256').update(rgba).digest('hex')!=='5f0bd289d43047c80022d2d213bfdd796ce762cb11551ee587e7df9708e00bac')throw Error('托盘解码与独立核验的RGBA不同');
writeFileSync(root+'pomodoro-tray.rgba',rgba);
const files=readdirSync(root+'original').sort().map(name=>{const b=readFileSync(root+'original/'+name);return{path:'original/'+name,url:name==='Rubik-Regular.ttf'?'https://fonts.gstatic.font.im/s/rubik/v31/iJWZBXyIfDnIV5PNhY1KTN7Z-Yh-B4iFV0Uw.ttf':name==='rubik-font.css'?'https://fonts.font.im/css?family=Rubik':name==='pomodoro.ico'?'https://itime.fun/web/pomodoro.ico':'https://itime.fun/web/assets/'+(name==='main-style-excerpts.css'?'index-DITefphd.css':name==='main-window-constants-excerpt.js'?'index-BJrdQu13.js':name),kind:name.includes('excerpt')?'verbatim-excerpt':'deployed-asset',bytes:b.length,sha256:createHash('sha256').update(b).digest('hex')};});
writeFileSync(root+'SOURCE_FILES.json',JSON.stringify({entry:'https://itime.fun/web/',deployment:'index-BJrdQu13.js',entrySha256:'79ba25621b91084bb06fa4a3e78888eee28f03767949ac7049ab19b7dee5cf06',upstreamId:'itime.fun.pomodoro',license:'公开部署未附完整许可声明，按用户明确自用范围保留来源',files},null,2)+'\n');console.log('原图标6种、原CSS、来源'+files.length+'份');
