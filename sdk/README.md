# SoulDesk 普通组件接口 v1（G2）

组件为独立静态 ZIP 包，根目录放 `manifest.json`，资源放 `dist/`。[组件广场](https://github.com/Cosmocraft996/SoulDesk-Widgets) 提供正式组件源和构建入口，`tests/fixtures/` 提供协议验收示例。

已实现 `settings.get`、`settings.update`、`storage.get` 和 `storage.set`，均要求 `storage.instance`。`host.js` 完成来源与父窗口核对、消息通道握手、请求编号和普通请求的10秒超时；原生文件选择由用户完成或取消，不套用此超时。组件不直接调用Tauri。

```javascript
import { connectHost } from './host.js';
const host = await connectHost();
const settings = await host.request('settings.get');
await host.request('settings.update', { ...settings, title: '我的时钟' });
// 初始化界面成功后报告就绪；初始化异常应继续抛出，让宿主阻止失败包启用。
await host.ready();
```

宿主按会话绑定实例和容器；组件不能通过参数指定其他实例。移动、重新加载和销毁使旧会话失效。`settings.update` 替换完整配置，成功返回值表示 SQLite 已保存，失败保留旧配置；`storage.set` 同样替换当前实例的存储对象。

配置结构支持 object、properties、additionalProperties=false、required，以及 boolean/string/number/integer 基本属性；属性支持 default、title、minimum、maximum、maxLength。配置结构与配置各限16 KiB，属性最多64项；当前源码将实例存储对象上限扩展到4 MiB（按完整JSON的UTF-8字节计），额外声明`storage.document`时为32 MiB，试加载仍只写内存。minimum/maximum仅用于数值，maxLength仅用于字符串；约束须有正确类型且范围不能倒置。旧宿主的实例存储上限仍为16 KiB；组件须处理保存错误并保留输入，成功响应后才提示已保存。

默认值由宿主显式生成，导入时必须能产生有效的新实例配置；必填项需要合法默认值，不能引用未知或重复属性。这个约定比 JSON Schema 自身的默认值注解更严格。integer 接受 `1.0` 这样的整数值，限于 JavaScript 安全整数范围 ±9007199254740991；普通配置数值不用于金融价格。

包限制：压缩大小 20 MiB，展开大小 50 MiB，最多 256 项。宿主拒绝越界、绝对路径、符号链接、Windows 保留名及不区分大小写的重名路径；只登记已完整验证的资源目录。

导入顺序：校验 → 不可变版本目录 → 隐藏试加载 → 事务启用。相同内容重复导入幂等，不撤销现有会话。升级仅接受更高数字版本；同版本的不同内容及降级明确拒绝。新版须兼容所有旧实例配置，只补充缺少的默认项，不静默重置。试加载分别使用默认配置及已有实例配置/存储的内存副本，只能写隔离数据；并发配置变化会中止本次切换。

新组件清单添加 `"loadCheck": 1`，并在初始化完成后 `await host.ready()`。宿主等待就绪且至少 800 ms 无捕获到的脚本/资源错误，10 秒内未就绪则保留原版。未声明该字段的 G0 草案包保留旧的成功接口请求判定。试加载不能证明未来任意时刻都不会出错；不隔离死循环，也不能识别被组件自己吞掉且谎报成功的错误。

启用时保存切换前的实例配置/存储快照并保留旧版本文件；事务失败不改变原登记。新会话绑定不可变资源摘要，旧页面不能在升级后取得新版会话。取消组件登记须先移除其实例，历史版本文件不会自动删除。

页面资源采用相对路径，脚本必须为外部文件。允许HTML/CSS/JS/MJS/JSON、常见图片、字体、TXT/MAP及本地mp3；拒绝系统脚本、动态库和可执行文件。`dist/__souldesk_runtime.js`是宿主保留名。默认禁止内联脚本、网络连接、嵌套框架、弹窗、表单导航和宿主命令；下面的本地库与网页嵌入能力仅按声明放开指定策略。宿主保留文档声明并注入错误监测脚本。组件是用户信任的本地代码，容器不能承诺隔离页面死循环或网页进程崩溃。

Windows 默认组件来源为 `http://widget.localhost`；SDK 从宿主提供的入口 URL 取得准确的宿主来源，不向通配符发送有权限消息。实际来源行为需结合安装版验收记录理解。

在项目的 Codex 聊天中选用 [`$souldesk-widget`](../.agents/skills/souldesk-widget/SKILL.md)，描述组件的显示内容、交互和需要保存的数据。例如：

> `$souldesk-widget` 做一个 Prompt 收藏夹。支持新增、编辑、搜索、复制和确认删除，收藏按实例保存在本机。打包成 ZIP 并在隔离实例中导入验证。

从广场仓库运行 `python scripts/widget.py prompt-favorites` 得到示例 ZIP。通过宿主管理窗口「导入本地 ZIP」登记后从组件库添加。开发验证用 `pnpm dev` 默认的隔离数据。

## 可选本地共享库（本轮0.5.0源码）

七款预置的0.2.0版本新增`storage.shared`：思维导图、倒计时、倒计时列表、生日列表、待办、大号日历、番茄钟。默认仍使用原`storage.instance`私有数据；这里只提供本地共享，原云同步未实现。宿主按固定预置身份限制为mindmap/countdown/birthday/todo/pomodoro五种数据类型，不允许跨类型连接。

| 方法 | 参数与行为 |
|---|---|
| `library.status` | 无参数；返回`{mode,binding,readRevision}`，binding为当前库元数据或null |
| `library.list` | 无参数；仅列同类型元数据及consumers，不返回文档内容 |
| `library.create` | `{name}`；普通文档创建独立副本；待办/番茄钟原生源在同一事务复制并连接，返回`reloadRequired:true` |
| `library.bind` | `{id}`；UUID连接或null断开，返回`{reloadRequired:true}`并使该实例所有旧会话失效 |
| `library.rename` | `{id,expectedRevision,name}`；版本匹配后改名 |
| `library.remove` | `{id,expectedRevision}`；仅允许删除没有实例连接的库 |

元数据含`version/id/kind/name/revision/updatedAt/consumers`。名称1～80字，最多64个库；导图内容最多32 MiB，日期类最多4 MiB/512项。数据与元数据在SQLite中分开存储，状态/目录轮询不读取私有大文档或共享内容。所有操作复用正式会话、窗口及清单校验，隐藏试加载拒绝`library.*`，预览原storage仍只写内存副本。

连接后原`storage.get/set`形状不变，宿主自动路由共享内容。get记录当前会话读版本，set在事务中比较该版本；其他实例已更新时返回`REVISION_CONFLICT`，保留旧库及页面草稿，状态轮询不会推进读版本。写成功后才推进游标。内容、元数据、视图偏好一起提交，任一失败回滚。修改已有库名也增加版本，原页面应重新载入后继续编辑。

倒计时只共享events，selectedId和sort按实例+库独立；导图只共享documents，preferences按实例+库独立；生日共享birthdays。连接不覆盖原私有存储，断开恢复原数据；删除实例清自身绑定/偏好，库继续保留。普通文档新建副本后若要连接，应显式调用bind并重新握手加载；原生源的新建返回reloadRequired时直接重载，不能使用失效会话再次bind。普通包升级仍须用户选择，不自动覆盖已登记0.1.0或重置配置。

共用面板只查询元数据并提示版本变化，不自动重载或拉取内容覆盖草稿。切换前等待保存队列，编辑窗口打开、保存失败或导图未保存时拒绝切换；用户可自行保存/导出，或明确选择“放弃未保存草稿并重新载入”。删除共享内容的确认必须说明会影响其他连接实例。该能力不代表实时协同编辑。原生待办/番茄钟的数据源与后台约定见下节。

### 原生待办与番茄钟共享源

待办和大号日历共用todo类型；番茄钟使用独立pomodoro类型。只有固定预置身份同时声明storage.shared和对应todos.manage/focus.pomodoro才可切换。库内容继续存放在原生业务文档中，由todo.get/change或pomodoro.get/change读取/修改；storage.get/set仅保留当前实例的辅助数据，不能覆写原生文档。主题等settings仍按实例独立。

原生binding另含contentRevision：元数据revision仅用于改名/删库，原生内容版本沿用业务revision；readRevision记录该会话最后成功的业务读取/修改版本。轮询status/list不读大文档、不推进游标。业务change仍要求expectedRevision和原UUID幂等回执，改名不自动放行旧业务草稿。

切源只运行当前选中源，原私有数据保留并暂停；断开恢复私有。番茄专注/休息保留已用时间，分别为pause/rest-pause，恢复须显式start，休息暂停start继续休息；新库复制场景、历史、设置和当前选择，清空活动周期/通知/请求回执，不自动开始。多个实例共用同源只调度一次；最后消费者离开才暂停库，其他实例仍连接时继续；隐藏或关闭视图不停止当前源后台。删除实例保留共享库，删除未连接库才删除对应业务文档。

切换和原生调度以native_actions→Sessions→SQLite的顺序协调；递送期间不持SQLite锁。暂停重置通知nonce，旧通知按钮不能修改恢复后的源。创建与连接在一个事务完成，写失败保留原源与会话。主视图切源会撤销同实例所有旧会话并广播重载；有其他正式详情会话时先拒绝，提示保存并关闭详情后从主视图切源，详情不提供切源按钮。当前视图有编辑/删除确认、未保存设置、保存失败或结果未确定时也拒绝切源。删除确认须说明共享内容影响全部连接实例。

真实双窗口/系统通知/旧版本非空升级仍待手动；本地共享库不等于原云同步完成。

## 和风天气能力（本轮0.5.0源码）

声明`weather.read`后，正式展示会话可调用以下接口；隐藏试加载全部拒绝。方法不接受实例标识、任意URL或请求头，只操作当前实例。

| 方法 | 参数与返回 |
|---|---|
| `weather.status` | 无参数；返回`{configured,enabled,apiHost}`，不返回密钥 |
| `weather.configure` | `{apiHost,apiKey?,enabled}`；首次及换Host必须提供自有Key，同Host留空保留；成功返回同上状态 |
| `weather.clear` | 无参数；只清当前实例凭据 |
| `weather.get` | `{kind:'now'\|'daily'\|'indices',location}`或`{kind:'lookup',query}`；返回`{source:'QWeather',fetchedAt,data}` |

API Host仅接受用户专属`*.qweatherapi.com`的HTTPS默认端口，不含路径或凭据。固定访问v7实时/3日预报、v7生活指数、Geo v2城市搜索；Key通过`X-QW-Api-Key`请求头发送，不进入URL。城市标识接受2至32位ASCII字母数字，天气读取另允许有效经纬度；搜索词编码为单个location参数。使用主程序网络设置、8秒超时、禁止重定向，压缩及解压各限2 MiB。

用户须自行开通服务并显式启用。密钥与启用状态独立于普通settings/storage，按实例使用Windows当前用户DPAPI加密落库；删除实例在原事务内清除密文。页面仅接收配置状态，保存失败保留旧配置；换Windows用户或密文损坏时明确报错，可清除后重配。请求完成后重验会话及当前凭据，配置变化的旧响应丢弃。

观测时间使用`data.now.obsTime`，不用fetchedAt代替；缺值和服务错误不得显示成0。大号页面按各接口独立保留旧值/提示失败，预报按fxDate与城市tz标日期，过期指数单独提示。天气页面每小时刷新，关闭页面后停止普通查询，不提供后台天气采集或警报。v7天气已被官方标记即将弃用，后续需适配v1；未实现空气质量接口。参见[鉴权](https://dev.qweather.com/docs/configuration/authentication/)、[API Host](https://dev.qweather.com/docs/configuration/api-host/)、[实时v7](https://dev.qweather.com/docs/api/weather/weather-now-webapi-v7/)、[城市标识与时区](https://dev.qweather.com/en/docs/resource/glossary/)。

## 行情与警报能力（宿主0.3.0起）

币安行情与价格警报包在[组件广场](https://github.com/Cosmocraft996/SoulDesk-Widgets/tree/main/widgets/market)维护。包可独立导入和升级，宿主负责行情与后台警报。

| 清单能力 | 接口 |
|---|---|
| `market.read` | `market.catalog`、`market.instruments`、`watchlist.get/set`、`watchlist.getItems/setItems`、`market.subscribe/unsubscribe/poll`、`market.preferences` |
| `market.chart` | `market.candles/openInterest`、行情详情页、`view.resizeMarket` |
| `alerts.manage` | `alerts.list/save/delete/history/historySettings/historyClear/retry` |

行情键为 `{exchange:'binance',market:'usd_m',symbol:'BTCUSDT',priceKind:'last'|'mark'}`。价格、阈值和步长均为字符串。`watchlist.set` 替换本实例有序列表，最多20项，后台持续保留Last/Mark消费者；规则是独立宿主对象，不随实例删除。

现货使用 `market:'spot'`，仅支持 `priceKind:'last'`。`market.instruments` 合并两个独立目录并返回 `market`、`symbol`、`baseAsset`、`quoteAsset`、`contractType`、`status` 和 `tickSize`。`watchlist.getItems/setItems` 读写 `{market,symbol,alias?}` 的有序数组，最多20项，兼容旧有纯合约列表。订阅快照新增 `marketStatus` 和以 `market:symbol` 索引的 `marketTickers`，保留旧 `tickers` 的永续索引。新鲜报价的 `direction` 为 `-1/0/1`，首次或恢复后尚无有效前一笔时为 `null`。

`market.candles/openInterest` 接收 `{key,interval,limit,endTime?}`，最多500项，返回 `{key,interval,rows}`。K 线提供开高低收、开收时间、成交量，标记价的成交量为 `null`。未平仓量仅用于 USDT 永续，最短5分钟，按交易所提供的时间点返回 `{time,value}`。请求经宿主网络配置读取固定币安公开接口，共用短期缓存和并发控制。历史数据仅用于展示，警报仍由实时新鲜报价驱动。

`market.preferences` 原子更新已声明的 `layout`、`rangeHours` 或 `maPeriods` 字段。`view.openDetails({page:'market-chart:spot:BTCUSDT'})` 或 `market-chart:usd_m:BTCUSDT` 打开该实例的独立行情详情。`view.resizeMarket({size:'small'|'medium'|'large'})` 只接受固定尺寸并按当前主组件归属保存布局。后台价格规则没有总条数上限，同一来源的规则共享相同订阅，市场连接仍受独立源流容量限制。

```javascript
const stop = await host.subscribe([
  { exchange: 'binance', market: 'usd_m', symbol: 'BTCUSDT', priceKind: 'mark' }
], snapshot => {
  // snapshot.quotes 包含完整行情键、源时间、ageMs和quality。
  // 缺少报价或quality不是fresh时，界面显示等待/上次报价，不能冒充实时价格。
});
await stop(); // 仅释放当前会话的本次临时订阅。
```

SDK经消息端口接收带订阅标识与递增序号的事件；宿主容器最多每秒4次拉取合并快照并转送。页面计时器只更新展示，行情连接和规则判断在Rust后台。会话销毁后临时订阅回收，不影响持久自选或规则。

`alerts.save` 参数为 `{rule,expectedRevision}`。新建 `revision=1`、`expectedRevision=null`，编辑传旧版本号，并把 `revision` 加 1。过期编辑明确冲突。`alerts.list` 返回 `{rule,state}` 列表。

`condition` 默认 `reach`（到价），首次启用或触发参数变化后，第一笔有效报价已满足时提醒一次。重复到价在冷却结束后按新鲜报价继续判断，`cooldownMs:0` 按 5 秒执行。选填 `cross`（穿越）时，`direction` 指定方向且 `cooldownMs` 须为 0。穿越在同一连接代次的连续有效报价中判断，首笔和重连后首笔仅建立基准。仅改名称或通知策略保留触发状态。

`notification: true` 开启弹窗，`notificationChannel` 默认 `windows`，也可选 `island`。灵动行情铃声由宿主设置统一管理，旧 `sound` 字段保留用于读取已有规则。`alerts.history` 使用 `offset/limit`（最多 100），返回触发原因及递送结果。灵动行情事件的 `sound: 'island'` 表示由灵动通知管理。`alerts.retry` 接收事件 ID 及 `channel: 'notification'`，对失败、结果不确定或过期的弹窗累计最多尝试 3 次。`sent` 表示系统调用成功或灵动队列已受理。

`alerts.historySettings` 传 `{}` 读取全宿主保留设置，传 `{days:正整数}` 修改保留天数，传 `{days:null}` 关闭自动清理，返回 `{days}`。默认 3 天。`alerts.historyClear` 传 `{}` 清空当前触发历史并返回 `{cleared}`，规则和正在进行的通知递送继续有效。

隐藏试加载只接收隔离配置和存储。组件应在 `host.preview` 为true时跳过真实行情/警报和公开内容，先完成静态界面初始化，再 `await host.ready()`。本地表单的提交事件已允许；内容安全策略仍禁止表单导航和网页直接请求外部接口，HTTPS图片仅向声明内容能力的包开放。确认交互使用HTML `dialog`；容器不开放原生alert/confirm/prompt。

## 公开内容与网页链接（当前0.5.0移植源码）

`content.read` 开放 `content.get`，参数为 `{source}`；source限于bilibili、bangumi、bilibili-rcmd、cls-telegraph、douyin、tou-tiao、weibo、zhihu、forex及metal。bilibili-rcmd额外接受lastShown（上游视频标识列表）；forex要求from/to，两者限CNY、USD、JPY、EUR、KRW、AUD、HKD、PHP、MYR、SGD、THB、VND、IDR、RUB。metal仅请求固定BiQuote四种合约，原始数字严格转为十进制字符串，保留来源、交易所、市场状态、实际报价时间、过期及源时效；非法身份、缺失价格或超出精度直接失败。不能传任意URL、请求头、Cookie或请求体。宿主使用现有网络设置，GET读取固定接口，禁止重定向，超时8秒，响应上限2 MiB；返回 `{data,source,fetchedAt}`。页面负责检查业务返回码与响应结构，失败保留同一内容源/币种对的上次数据并明确显示失败及旧数据状态，不替代为空榜或虚构汇率。不会把这些公开内容接入币安行情或价格警报。

`itime-calendar`只接受`{source:'itime-calendar'}`，请求原iTime固定公开接口`https://itime.fun/api/v2/event/public/calendar`，不接收年份、月份、URL或其他额外参数。data保留原`{code,message,data}`结构；前端核对业务码、日期唯一性/有效性、文本上限和status。休/班只取源status，未提供的日期不推测安排；fetchedAt为本机读取时间，不代表源数据更新时间。此来源需要含它的新主程序，沿用content.read的会话检查、代理、禁止重定向、8秒和2 MiB边界。

组件通过`content.source.<source>`声明允许的`content.get`来源，仅接受登记表中的完整能力名。来源对应表见[公开内容](../docs/18-公开内容来源与图片权限.md)及[金融组件](../docs/19-架构审计收尾.md)。宿主在读取缓存或请求网络前校验来源，等待返回后再次核对会话。调用参数、响应和刷新行为沿用原接口。

`images.https`独立允许浏览器加载HTTPS图片，继续使用原域名范围。策略按资源对应的固定包版本计算，图片由WebView使用系统网络加载，当前不保证跟随应用内自定义代理。读取JSON仍走声明来源对应的宿主接口。浏览器同源和data图片及其他基础策略保持。

新版汇率组件用 `content.get({source:"forex-rates",cache:true,forceRefresh?})` 获取公开参考汇率，沿用 `content.source.forex`。宿主使用 Frankfurter v2 固定的美元汇率表和币种目录，取两者均支持的货币，返回 `{data:{base:"USD",quotes:{[code]:{rate,date}}},source,fetchedAt}`。汇率以十进制字符串返回，日期为对应观察值的 `YYYY-MM-DD`，基准美元固定为 `{rate:"1",date:null}`。缓存按网络设置隔离，共享 30 分钟，手动刷新可绕过已完成的缓存。旧包仍可调用原 `forex` 币种对接口。

旧`content.read`继续包含原固定来源及HTTPS图片许可。旧权限不能与`content.source.*`或`images.https`混用。使用细分权限的新包须先更新宿主，再独立导入或显式更新预置组件。见[18-公开内容来源与图片权限](../docs/18-公开内容来源与图片权限.md)。

黄金另有固定来源`gold`和`gold-trends`，均只读118.AU9999，不接收合约或URL参数。前者返回精确价格字符串、统计值或null、原报价毫秒时间戳，后者返回原分时时间及精确价格数组。宿主保留这两个请求的原x-custom-referer固定头，不接收组件传入的请求头；其他内容来源沿用无自定义头的请求。报价/图源错误分别处理，不把缺失统计值显示为0，也不把图源失败当成无有效报价。

`a-index`接受原000001/399001/399006三个`code`，返回百度完整指数身份、价格/涨跌字符串及原状态/时间文本。`stock-search`接受最多160字节的`query`，宿主编码后查询固定百度地址，返回保留代码与市场的候选；无效价格为null。`stock-minute`接受`code`及白名单中的东方财富`market`数值标识，固定原分时字段，返回精确昨收、图点、时间文本和原状态；返回代码和市场必须匹配，页面不能自选地址或提供请求头。

完整股票另使用`stock-search-eastmoney`及`stock-quote`两个固定公开源，同属`content.read`、使用主程序网络。前者接受query并返回原Name、code和marketId，保留同代码不同市场；后者接受code与market，固定按secid请求东方财富报价，返回`{code,marketId,name,decimal,price,previousClose,sourceTime,identityBasis}`。价格为fltt=2已带小数的精确字符串，不再缩放；decimal来自f59，实际小数位不得超出声明值。sourceTime为f86原秒时间或null，读取时间在外层fetchedAt单列。公开响应无市场字段，identityBasis明确为request-secid，只验证返回代码并将市场绑定固定请求，不伪称来源含市场字段。路径、地址及请求头不能由页面更改。

完整股票的自选和手工持仓用`storage.instance`按实例保存为`{version:1,stocks:[{code,marketId,name,holdingPrice,holdingShares}]}`；成本/份额为非负十进制文本，空字符串表示未配置，最多50项，以市场+代码去重。收益只在页面以BigInt十进制计算，不参与行情警报或交易；成本为0不计算，缺报价不当0。收益不含税费、分红和汇兑，不进行跨货币总计。原迷你曲线是公式示意，页面明确标注，不称真实分时。

`coin-index`接受原14项目录中的`instrument`，只读固定OKX公开指数路径，返回`{instrument,price,reference,basis,timestamp}`，价格为严格十进制字符串。basis为open24h、sodUtc8或null，必须分别标注；timestamp为来源毫秒时间，不是接收时间。此来源的JSON请求按当前会话所属实例设置选择网络：`proxyMode=inherit|direct|http|socks5`，另有proxyHost、proxyPort；从数据库读取，不接受消息指定其他实例，使用独立代理不改全局配置。其他内容来源仍使用主程序网络设置。隐藏试加载拒绝联网，真实代理及界面验证单列。

普通公开内容可通过`host.request('content.get',{source,cache:true,forceRefresh:true})`显式使用宿主内存缓存；`forceRefresh`可省略，表示常规读取。仅适用于九个热点来源及`itime-calendar`，具体时长见[16-公开内容请求缓存](../docs/16-公开内容请求缓存.md)。手动刷新绕过已保存结果，可以共用在途的新请求；失败不缓存、不返回旧结果充当成功，命中时保留原`source`和`fetchedAt`。来源、参数及网络设置不同不共用，黄历跨日失效。旧包不传`cache:true`时保持直接读取；这一选项不改变能力授权，也不作用于行情或价格警报。新日历包兼容旧宿主时只针对明确的元数据不支持错误退回原请求，网络失败不自动重试。

`content.source.coin-index`和旧`content.read`也允许`host.request('content.peek',{source:'coin-index',instrument})`。该方法仅返回实例、币种和当前网络配置匹配的进程内展示快照，无记录时返回null，无网络请求。快照沿用原data/source/fetchedAt，并附`cached:true`和最多48个实际`{timestamp,price}`采样。最多保留128个实例，删除实例或退出后清理。页面重建时先恢复快照再报告ready，随后后台更新。正式会话可读取，隐藏试加载拒绝。

`links.open` 开放同名方法，参数为 `{url}`，用于用户点击后打开系统默认浏览器；仅接收无凭证、无自定义端口的HTTPS公开域名，拒绝本地地址、文件及脚本协议。当前Windows使用参数数组交给系统URL处理器，不通过命令解释器。内容与链接能力均核对当前实例会话和声明权限，隐藏试加载拒绝调用，试加载页面应明确显示“不联网”。这些能力仅完成源码/核心检查，真实联网与桌面交互以13的证据为准。

`links.openWeb` 需单独在清单声明，参数为 `{url}`，接受完整的 HTTP 或 HTTPS 网址，支持主机名、IPv4、方括号中的 IPv6 和自定义端口。用户点击后由 Windows 默认浏览器打开，试加载不调用。宿主拒绝带凭证、控制字符及其他协议的地址。网站收藏夹将未写协议的 IP 或带端口地址按 HTTP 保存，显式填写的 HTTPS 保留。

网站收藏夹使用 `links.metadata`，清单需独立声明此权限。参数为 `{url}`，返回 `{title, icon}`，其中 `icon` 为图片数据地址，两项缺失时分别返回 `null`。宿主直接访问公网 HTTP/HTTPS，支持用户填写的端口。跳转或网页声明的图标使用自定义端口时，必须与当前请求或网页同源。每次连接使用核验通过的解析地址。请求仅采用 GET，无 Cookie 或代理。网页读取上限 512 KiB、超时 4 秒，图标上限 128 KiB、超时 2 秒。试加载禁用此能力。组件将图标缩为 32 像素后随收藏保存，获取失败时仍可使用域名、手填名称和占位图标。

元数据响应可包含 `status`，取 `complete`、`partial`、`unavailable` 或 `verification`。Cloudflare 的 `cf-mitigated: challenge` 响应进入验证状态，即使 HTTP 状态为 200 也忽略其网页标题。

`links.verify` 是 Windows 网站收藏夹专用权限。用户点击后，`links.verify.start({id,url})` 打开临时窗口，`id` 是本次尝试的 UUID。`links.verify.status({id})` 仅对同一实例返回 `pending`、`ready` 或 `closed`，`ready` 附 `{title,icon}`。用户完成验证后点击窗口菜单，宿主从当前同源、加载成功的文档读取标题及 WebView2 图标，随后关闭窗口。网页在独立无痕视图内运行，宿主消息桥只由本机组件使用。`links.verify.close({id})` 关闭并清理当前尝试。窗口在十分钟后自动关闭。其他站点跳转或验证失败时，可用系统浏览器打开并手动保存。隐藏试加载不触发这些操作。


## 原生实时内容（当前0.5.0移植源码）

`content.live`能力开放`host.request('content.live',{source:'coin-currencies'})`。目前只支持原主流虚拟币的固定OKX网页频道及原14币种，调用方不能指定地址、频道、币种列表、请求头或凭据；它与`coin-index`公开指数的口径不同。网页频道依据固定源码和原生实测维护，本次公开文档检索未找到对应说明。

请求立即返回当前原生快照`{source,fetchedAt,data}`，首轮为连接中，不等待远程握手。data包含provider/source/channel、connectionId、连接epoch、connecting/connected/retrying状态、错误、quotes及currencyErrors。报价包含currency、精确十进制price/reference、源timestamp、本机receivedAt和所属epoch；reference为空时不计算零涨跌。epoch变化或断线时保留报价但视为旧值，connectionId变化意味着不同的连接，页面须撤下旧连接显示。source时间与本机接收时间不能混用。

原生连接按有效网络配置共享，会话分别持有；关闭或替换一个会话不影响其他消费者，最后会话消失后取消连接，退出停止所有普通实时连接。此时只在进程内保留最近8种网络口径的末次报价，不保留无人使用的套接字。重新打开后使用独立连接状态恢复原报价，保留timestamp/receivedAt并先标连接中；新连接/代次或错误期间旧报价仍按旧数据显示，直到新来源更新。不同网络口径不混用缓存，退出清空。窗口释放/主动关闭会话时立即清理，其他会话变更由原生一秒检查回收。20秒ping、35秒接收超时、1～30秒重连；传输消息上限512 KiB，报价JSON处理上限64 KiB。网络设置复用coin-index的实例代理字段，下一次读取按新配置切换；隐藏试加载拒绝调用。此能力只读网页币种报价，不接入币安警报。

## 本地文档与编辑器（当前0.5.0移植源码）

新版按需声明`documents.open`和`documents.save`，分别调用同名方法。旧`documents.user`兼容原打开、保存和拖放行为，新旧声明不可混用。参数包含普通文件名`fileName`和扩展名数组`extensions`，打开可带`multiple:true`，保存须带base64编码的`data`。每次调用显示原生文件对话框，由用户选择文件。打开最多32个文件、合计16 MiB，返回`[{name,mime,data}]`。保存上限32 MiB，成功返回`{name,size}`，采用同目录临时文件写入后替换。取消返回null，选择后继续核对会话和权限。支持格式及错误规则沿用原协议。

`documents.drop`控制正式桌面或侧栏窗口是否启用浏览器文件拖放，面板按其中可见组件计算窗口策略。管理窗口和隐藏试加载沿用原生拖放处理。拖放数据由用户当前操作交给页面。实际Windows拖放仍待验证。

本地资源细分模式须声明`assets.scoped`，再按实际需要声明下列能力。模式标志本身不增加资源权限，并显式禁止未声明的工作线程，避免浏览器回退到脚本的同来源许可。

| 能力 | 浏览器侧增量 |
| --- | --- |
| `styles.inline` | 库生成的内联样式 |
| `images.blob` | Blob图片；原有同源及data图片不变 |
| `assets.fetch` | 本地资源同来源的fetch/XHR读取，例如字体子集数据；不开放任意外网 |
| `runtime.wasm` | WASM编译，不因此允许动态JavaScript |
| `runtime.eval` | 动态JavaScript；浏览器同时隐含允许WASM，不能单独禁止 |
| `workers.local` | 同来源工作线程 |
| `workers.blob` | Blob工作线程 |

普通外部脚本/CSS、同源字体及静态图片继续遵循原基础策略。DOM直接设置`element.style.width`等样式属性无需`styles.inline`；动态style标签及样式文本受CSP约束。同来源按浏览器来源划分，不是按包目录隔离。细分能力不授予宿主文件、网络、剪贴板或后台接口，其他业务能力附带的既有浏览器许可保持。

旧`assets.local`继续按原样兼容：内联样式、Blob图片、同来源读取、同源/Blob线程及动态代码/WASM全开。不能与`assets.scoped`混用；细分项缺少模式标志会拒绝导入。资源使用各自包摘要绑定的清单，旧资源不会继承新版权限。新版五款组件需要包含本次能力登记的主程序，旧宿主会明确拒绝未知能力；应先升级宿主，再显式升级组件，不自动改写旧包或实例数据。能力分配与验证见[17-本地资源权限细分](../docs/17-本地资源权限细分.md)。

`web.embed`允许HTTPS子页面及演示/全屏，保留禁止弹窗、顶层导航和表单导航；远程子页面没有宿主消息端口。`clipboard.read`与`clipboard.write`分别映射浏览器剪贴板权限，仍受WebView的安全上下文、用户操作和系统权限约束。隐藏试加载不授予剪贴板、演示或全屏权限。组件不能指定其他实例取得权限；旧版本资源不会取得新版清单的能力。真实文件对话框、剪贴板及嵌入行为仍待桌面验证。

`media.directory`允许使用`documents.open({directory:true})`选择本实例的只读图片文件夹，不要求`documents.user`。用户取消返回null并保留原授权。宿主私有保存完整目录，返回`{name,revision,files:[{name,size}]}`，只列直接子文件中的jpg/jpeg/gif/png/webp；每目录最多8192个条目、4096张图片。`media.list`恢复或刷新当前列表，未授权返回null；`media.read({revision,name})`按需返回`{name,mime,data}`，单图上限16 MiB，data为base64，不接收路径或其他实例标识。拒绝越界名称及指向授权目录外的链接；目录重选后拒绝旧revision，耗时读取前后都核对会话及当前目录版本。`media.clear`及删除实例只清除授权，不删除照片。原生选目录、真实显示及重启界面仍待验收。

## 番茄钟后台（0.5.0本轮源码）

`focus.pomodoro`允许正式会话调用`pomodoro.get()`和`pomodoro.change(args)`，预览拒绝。实例由窗口和令牌绑定，不接受实例、时间、路径、URL或自定义通知文字。`get`仅接受空参数或空对象，返回`{version:1,revision,initialized,sampledAt,config,scenes,history,currentSceneId,phase,remainingSeconds,progress,notice,message}`。场景时长由剩余历史求和；时间戳为核心UTC毫秒，历史duration为有效专注整数秒。原生计时、记录和完成提醒不依赖视图存活。

`change`必须有UUID `requestId`、非负`expectedRevision`及`operation`。操作为`initialize`（templates数组：reading/coding/working/relaxing，可为空）、`scene.save`（scene含UUID id/name/icon）、`scene.delete/select/history.delete`（id）、`reorder`（全部场景ids）、`configure`（完整config）、`start/pause/finish/reset`（无额外参数）。不允许不匹配字段。名称1～120字、图标1～32字符；最多128场景、8192记录、完整后台文档4 MiB；超限拒绝，不删除旧历史。当前周期绑定开始场景，活动阶段切换/删除该场景须先结束或重置；删除场景会一并删除其历史，视图须先确认。请求成功后才更新界面；不确定结果使用原UUID和原参数重试，核心保存最近128次回执；冲突重新读取。

config为`{pomoTime,shortBreakTime,isAutoNext,isSoundEnabled}`，默认30/5分钟及两个true；整数范围分别5～60、1～30。阶段stop/running/pause/waiting/resting：到时等待确认，finish才保存有效专注并休息；少于60秒不记历史并返回message。暂停不计时，继续重建锚点；重启继续当前阶段，休眠只推进当前阶段，自动下一轮从当前时刻开始，不补造多轮；回拨保留累计，设置变化不倒扣。

完成提醒发送前事务预约并复核周期；`notice.notification/sound`区分waiting/sending/accepted/failed/unknown，另有dismissed或disabled。accepted只表示系统接受调用，不代表看到或听到；重启发现sending记unknown且不自动重发。通知按钮核对实例、周期和通知凭据，旧周期不得操作新周期。原WAV由Windows原生播放，关闭声音不修改系统音量；实际通知、播放和按钮仍需实机验证。实例删除事务清理自己的后台数据。

场景、设置和统计页面可调用`view.openDetails({page:'pomodoro-scenes'|'pomodoro-settings'|'pomodoro-history'})`，要求当前正式会话声明`focus.pomodoro`，只接受page字段。复用固定详情窗口；不开放任意URL、跨实例或窗口参数。原云账户/同步未实现；可选本地共享库见前文。贴边隐藏、独立托盘与强制置顶按已批准的显式启用建议后续实施。

## 待办与提醒后台（0.5.0本轮源码）

`todos.manage`只允许正式会话调用`todo.get()`和`todo.change(args)`，试加载拒绝访问。实例由当前窗口与令牌确定，参数不接受其他实例、路径或任意通知。每实例数据独立，`todo.get()`仅接受空参数或空对象，返回`{version:1,revision,todos,reminders,sampledAt}`；提醒记录含`todoId/at/delivery/error`，不暴露通知按钮凭据。旧宿主需要升级。

变更含UUID `requestId`、读取时的`expectedRevision`和`operation`。`save`携带`todo`；`complete/reopen/delete`携带`id`；`reorder`携带全部未完成事项的`ids`，不混入历史或重复标识。todo保留原UUID、title、可选dueDateTime/reminderDateTime/startDateTime、isReminderOn、recurrence、importance及order，创建/修改/完成时间由核心维护。内容1～4096字、最多512项、完整后台数据4 MiB；ISO日期严格校验，提醒时间必须晚于保存时刻。重复支持原界面的每日/每周/每月，已有每年规则可读取；其他规则拒绝。冲突和失败不更新原数据；同一请求确认前用原参数及requestId重试，核心保留最近128次去重记录。

完成事项及生成下一次重复处于同一事务；恢复历史不删除已生成的下一次事项。重复按本机日历推进，月31日跳过不存在的月份，下一次严格晚于当前时刻与原到期时间；不存在或不唯一的本地时间明确失败。提醒随新到期日保持原相对时差，已过期的新提醒关闭，前端应如实显示开关。配置/事项写入成功后才生效；待办与大号日历可选同一本地共享库，原云账户/同步仍未实现。

核心按秒检查已保存提醒，独立于组件视图和会话。发送前事务预约，每条实际发送前重新读取时钟、有效期和事项签名；超过目标时刻30秒后记expired，不补发旧提醒。`waiting/sending/accepted/failed/unknown/expired/dismissed`区分待调度、待确定、系统接受、失败、上次未确定、过期和知道了；accepted不代表用户看到。沿用Windows系统通知及“知道了/已完成”按钮，显示时长由系统管理。按钮核对实例、事项签名、通知标识及有效期，不能完成编辑过、删除或其他实例的事项；移除实例事务清理自己的后台数据。真实通知显示与按钮点击仍未验。

## 打工进度与下班提醒（0.5.0本轮源码）

`reminders.labor`允许正式展示会话调用`labor.get()`或`labor.get({})`，首次读取注册实例后台。方法不接收时间、实例、通知文字或其他参数；实例设置须包含`startHour/startMinute/endHour/endMinute`四个整数，小时0～23、分钟0～59，上下班不能相同。返回`{sampledAt,startAt,endAt,isoWeekday,percent,remainingSeconds,delivery,lastError}`；区间使用核心Windows动态时区与当前本地日历转换，时间戳均为UTC毫秒，进度向下截一位并限制0～100，weekday为当前本地星期一1至星期日7。

夜班跨零点接续前一天的上班，结束后保持上一班完成至下一班开始；日班每天使用当天区间。转换检查本地时间回转，不存在的夏令时跳时时刻明确失败；重复时刻采用Windows时区转换规则。首次正式读取后，隐藏或销毁视图不停止后台；预览不注册。倒计时在班次进行中且距结束0～300秒时首次发送，临近下班才启动/恢复最多发送当前一次，结束后不补发旧提醒。

下班倒计时沿用Windows系统通知，初次显示一次后用数据绑定及递增序号静默更新真实分秒；清除、失败或递送未确定后，同一结束时刻不重新弹出。重启只更新已存在的通知，不再次显示同一班次；时间倒退、区间改变或结束清理旧通知，实例删除事务清状态并排队只清自己的通知，通知自身设置结束过期。后台保留最近32个目标结束时刻去重。`sending/accepted/failed/dismissed/unknown/stopped`区分待确定、系统接受、失败、通知不存在、上次未确定和已停止；accepted不代表用户看到。系统通知弹出时长由Windows决定，不承诺原灵动岛常驻画面或模拟百分秒。

闲置读取不重复写状态，活动倒计时按秒保存更新序号以便重启继续；当前仅完成Windows时间转换、正式权限/持久化及通知返回替身检查，真实通知显示/更新、桌面表情和设置点击仍待验收。该固定能力不开放通用日程、任意通知或脚本后台，需要包含此能力的新主程序。

## 服务器监控能力（0.5.0本轮源码）

`servers.monitor`仅授予正式展示会话调用`server.request({kind,connection,forward?,servers?})`；`connection`与可选`forward`均为`{url,token}`。URL为用户自己的HTTP/HTTPS监控端点，可使用本地或内网地址，不接收内嵌认证、查询、片段和空白；最多2048字符，token为1～4096字符的合法请求头值。接口可沿用宿主网络设置，但没有任意HTTP方法、请求头、请求体或系统查询字段。

kind固定为stats、info、pull或push。stats按原客户端POST `mem:*`、`currentLoad:avgLoad,currentLoad,currentLoadUser,currentLoadIdle,currentLoadSystem`和`fsSize:*`；info仅POST mem/cpu/osInfo三个固定字段。二者允许forward，将目标通过原`forwardUrl/forwardToken`交给用户选择的主服务器。pull固定GET端点追加的`/servers`，push固定POST同一路径的服务器配置数组；二者不接受forward，只有push接受servers。配置数组最多128项，并受4MiB请求上限限制。原token请求头与POST中的连接参数保持原协议，不开放任意systeminformation方法或命令。

读取返回`{data,receivedAt}`，receivedAt仅为本机完整接收的毫秒时间，不能冒称服务端采样时间。push返回`{accepted:true,receivedAt}`，只说明HTTP请求被接受，不证明对方已经持久保存。响应上限4MiB、总超时8秒、禁止重定向；非成功HTTP、格式、流式超限、网络及令牌错误均明确失败，不把地址、令牌或响应原文拼入错误。调用前后核对会话与能力，预览拒绝联网。

服务器组件配置仍使用4MiB实例存储，串行提交成功后才应用。原刷新间隔设置现在按每台1～3600秒（默认10秒）生效；轮询最多4并发、更改地址/令牌/主服务器后忽略旧响应，删除后释放自己的读取状态。主服务器同步包含连接令牌和元数据，界面明确说明，并只向用户指定的服务发送；本地保存与远端同步结果分开。当前本地HTTP夹具、静态模板与组件动作检查通过；真实服务、桌面、剪贴板和同步未验收，原服务端仓库链接404，没有替代服务部署。

## 喝水提醒能力（0.5.0本轮源码）

声明`reminders.water`后，正式展示会话可调用`water.get()`或`water.get({})`，首次调用在核心注册本实例提醒。`water.change({requestId,operation})`仅接受UUID请求标识和`add`或`reset`，分别对今天加一杯或归零；不接受实例、路径、通知文字或其他字段。响应包含`sampledAt/day/count/targetCup/enableReminder/interval/lastReminderAt/nextReminderAt/delivery/lastError`。毫秒时间用于间隔，day按Windows当前本地日期计算，历史按日和实例隔离。

本能力要求实例设置包含`enableReminder`布尔、`interval`整数5～100和`targetCup`整数1～20。隐藏试加载拒绝调用；组件视图隐藏、移动或销毁后，已注册提醒仍由核心调度，退出程序停止。到期且未达目标时最多发送一次固定喝水通知，按钮为关闭和喝一杯；显示时间由Windows管理。`delivery`的`sending/accepted/failed/unknown`分别表示待确定、系统接受、失败和上次未确定，accepted不代表用户看见。失败不逐秒重试，下个间隔再尝试；时钟倒退重新计时，休眠/重启不补发每一轮。

杯数先事务提交再响应；调用超时后应保留同一requestId重试，确认前阻止新变更。核心保留最近128个请求的去重记录，重复请求返回当前日期快照，跨日重试不对新日期重复加杯。通知按钮同时核对实例、当天、提醒标识及有效期，重复或过期点击不加杯；移除实例在同一事务删除自己的提醒与按日记录，不删除其他实例。实际Windows通知显示、按钮点击和桌面画面尚未验收。

## 本机监控能力（0.5.0本轮源码）

`system.metrics`允许正式展示会话执行同名方法，不接受参数或仅接受空对象，返回`{sampledAt,cpu:{percent,error},memory:{percent,error},gpu:{percent,error},details}`。percent是0～100的数值或null，null同时给出不可用原因；sampledAt为本机采样毫秒时间。没有任意系统信息、进程列表、传感器名称、远程设备或命令入口，隐藏试加载拒绝调用。旧宿主不识别此能力，需要更新主程序。

Windows CPU使用系统时间差分，内核时间包含空闲时间；多处理器组暂明确不支持。内存是物理内存使用百分比。GPU使用语言无关的Windows性能计数器，先按物理适配器/引擎汇总各进程，再取最繁忙引擎，不能视为原LibreHardwareMonitor首个Load传感器的同一口径。首次CPU/GPU需要两次采样，驱动未提供或读数失败返回null，不能伪造0%。所有实例共享最多每秒一次的缓存，无网页轮询时不采集；超过10秒无读取后，下次重新预热。采集在阻塞任务池执行，query句柄由原生析构释放。


打工能耗标识复用该接口与同一有效值/时效模型，以CPU×0.8＋内存×0.2按每20个百分点向上取整分为1～5档，分级使用未舍入的读数。CPU或内存任一无效时等级为缺失，另一个真实指标仍可显示；不可用数据不会显示绿色一级。它的名称、型号、字号和圆角仍按实例保存；两个组件包各自包含`dist/hardware-common/metrics.js`，相对模块保持目录层次，既有标准包校验不变。

监控面板增加固定的details字段：cpuLogicalCount为系统逻辑处理器数或null；memoryTotalBytes为物理内存总字节的精确字符串或null；gpuAdapter为占用率所选适配器的临时LUID/物理索引，gpuTemperature为`{celsius,error}`，驱动不提供时celsius为null。温度仅向同一适配器查询，不能用另一块GPU的温度补缺。GPU主温度与原硬件库的GPU Core传感器口径仍区分，不开放调用方指定的查询类型或驱动参数。

details.network为`{downloadBytesPerSecond,uploadBytesPerSecond,receivedBytes,sentBytes,error}`。速率为非负数或null，累计字节为精确十进制字符串或null；按所有已连接非回环接口聚合，虚拟网卡可能重复计数。首次、接口集合变化、计数器倒退及超过10秒的间隔不输出速率，下一次重新预热；已有累计量可显示。接口数量上限8192，返回表由原生释放，不向组件返回名称、IP、MAC或接口标识。所有字段继续受同一正式会话权限和一秒采样缓存约束，旧组件可忽略新增details。

## 文件收纳能力（0.5.0本轮源码）

`files.launcher`供用户信任的文件收纳组件使用，可列举已选择主目录的直接子项目、显示缩略图或系统图标，并请求系统打开已授权项目。此能力允许打开程序或快捷方式；组件本身仅在用户点击时发起打开。没有任意命令、启动参数、文件内容读取、重命名、移动或磁盘删除接口。旧宿主不识别此能力，需要更新主程序。

- `documents.open({launcher:'root'|'file'|'folder',revision})`：通过原生对话框选择主目录、额外文件或额外目录。初次revision为null，后续使用当前版本；取消返回null且保留原授权。该选择分支复用SDK已有的用户等待语义，不要求`documents.user`。
- `files.list`：返回`{revision,root,rootError,files,excluded}`。首次无授权时revision/root为null、列表为空。每个文件包含宿主标识id、名称、已授权项目的displayPath、目录标记、创建/修改毫秒时间、精确字符串size及unavailable状态；缺少时间返回null。主目录失败保留授权及错误信息，便于重新选择；额外项目继续列出。
- `files.icon({revision,id})`：按需返回`{width,height,rgba}`，rgba为base64编码的未预乘RGBA像素，尺寸最多96×96；Windows Shell优先缩略图，再使用系统图标。页面用本地画布转换为图片，不取得原文件内容。
- `files.open({revision,id})`：仅解析本实例已授权的标识，使用固定open动作及父目录工作位置，没有调用方参数；`{launched:true}`仅表示系统接受打开请求。
- `files.exclude({revision,id})`、`files.restore({revision,id?})`只改变本实例排除记录，省略恢复id表示恢复全部。`files.clear({revision})`仅清除主目录，额外项目保留。修改成功返回完整新快照；失败不提示成功。

主目录最多4096项、额外项目1024项、排除记录4096项，不扫描子目录；越界链接和旧版本标识拒绝操作。授权路径保存在宿主私有设置中；组件只可显示本实例已选择的路径，不能传任意路径调用。删除实例同时清除本实例授权，不删除原文件。会话、窗口及当前版本均需有效，隐藏试加载不能调用这些能力。原生选择、实际画面与关联打开仍待桌面验收。

## 正式桌面展示（宿主0.4.0起）

宿主注入的运行错误监测保留脚本异常、未处理拒绝和关键资源失败；图片错误延后一个任务检查，使组件的回退处理先执行。已换src/srcset、显式阻止默认错误或移除图片时不误判组件失败；最终图片仍失败且未处理时继续报告。该判断不代替实际图片、布局或交互验收。

实例的持久归属是 `desktop` 或 `panel`，独立显隐与全局隐藏分开。移动和隐藏先撤销主会话并卸载展示；配置、自选与有效规则继续保留。关闭管理或收起面板不影响桌面组件。管理窗口没有组件展示会话权限。

宿主在资源入口查询参数提供 `role=primary|details`。行情0.2.0在主展示使用独立紧凑布局；详情仍使用同一包、实例和业务后台。带 `alerts.manage` 的组件可主动请求 `host.request('view.openDetails', {page:'quotes'|'rules'|'history'})`，该方法不接受其他实例ID或任意窗口参数。详情按其窗口身份获得独立会话，不撤销主展示，也不复制规则；普通时钟/卡片不需要采用新接口。旧行情包忽略角色参数后仍展示其原页面；旧宿主未提供角色时，新包保留完整页面兼容入口。

可信鼠标意图由宿主全帧初始化脚本提供，不要求普通包重编译SDK；桌面输入适配在原生端再次核对物理键、命中、自有可见窗口和焦点，不向包开放任意前台或系统命令。面板折叠时卸载 iframe、关闭消息端口和展示轮询，恢复后重新取得新快照。


## 键盘演示

`keyboard.display`仅用于固定标准包`local.souldesk.widgetjs.key-stroke`。正式控制卡调用`keyboard.status()`或`keyboard.configure({enabled:boolean})`，其他字段拒绝；默认false，每实例独立保存。返回`{enabled,listening,suspended,lastError}`，已保存启用与实际监听分别显示，错误后由任一已启用实例显式重试。隐藏/移动控制卡不停止；隐藏全部暂停，恢复后重建；最后启用实例停用/删除或退出时停止，清空内存快照。

宿主固定创建`keyboard-{instanceId}`透明、无焦点、置顶、鼠标穿透的450×240底部叠加层。该视图经相同SDK桥接获得`keyboard-overlay`角色，只允许`runtime.ready/keyboard.status/keyboard.peek`；不允许设置、存储、配置或其他原生能力。`keyboard.peek()`只返回当前快照`{visible,modifierKeys,normalKey,capsLock,isCapsLockOn}`；键含`keyCode/name/isKeyUp`，普通无修饰文字不形成快照，也不生成键名。无输入历史、持久化、日志或上传。全局钩子仅在用户显式开启且显示层初始化成功后安装，多个消费者共用一份；真实Windows事件、穿透和焦点仍单列手动验收。

## 组件通知（`notifications.send`）

在 `manifest.json` 的 `capabilities` 声明 `notifications.send`。组件通过 `channel: 'windows'` 请求 Windows 系统通知，通过 `channel: 'island'` 请求灵动胶囊。「应用设置 → 通知」分别控制两个渠道，默认都开启。关闭所选渠道时返回 `suppressed`。每次请求对应单个渠道，同一业务事件确需同时使用两种渠道时，每路使用各自的请求 UUID。旧调用省略 `channel` 时沿用旧的 Windows 或灵动渠道偏好，缺少偏好时选择 Windows。

Windows 系统通知以文字呈现。需要胶囊内按钮、实时倒计时或来电交互时选择灵动通知。模拟来电同时声明 `notifications.call`，并由用户开启「允许小组件触发模拟来电」。发布到组件广场时，最低宿主版本填写实际验收过且支持相应参数的正式版本。

```javascript
// 页面初始化并报告 host.ready() 后，从实际业务事件触发。
if (!host.preview) {
  const result = await host.request('notifications.send', {
    requestId: crypto.randomUUID(),
    channel: 'island',
    kind: 'advance-countdown',
    title: '会议即将开始',
    body: '还有一分钟',
    targetAt: Date.now() + 60_000,
  });
  // result.status === 'accepted' 表示宿主或系统已受理，实际显示由宿主处理。
}
```

| `kind` | 参数 | 灵动通知行为 |
| --- | --- | --- |
| `info` / `success` / `warning` / `error` | `title`、可选 `body` | 短消息胶囊，省略 `kind` 时为 `info` |
| `reminder` | `title`、可选 `body` | 「关闭」或「打开组件」，宿主回到来源实例 |
| `advance-countdown` | `title`、可选 `body`、`targetAt` | 先显示预告，点击后显示实时倒计时 |
| `countdown` | `title`、可选 `body`、`targetAt` | 立即显示剩余时间，到期收起 |
| `call` | `preset` 为 `squidward\|tea\|pill\|study` | 使用内置头像与音频，支持接听重试或挂断 |

每种类型都需要 UUID `requestId`，同一事件重试应复用原 ID 与完整参数。标题不超过 80 字符，正文不超过 200 字符。`targetAt` 是未来一小时内的 Unix 毫秒时间，仅用于倒计时显示。`accepted` 返回 `id/channel`，关闭渠道、隐藏全部或来电开关关闭时返回 `suppressed`。试加载、缺少权限、错误参数或达到限额时返回错误。宿主在当前进程保留最多 5 分钟的回执，同一实例每 30 秒最多受理 5 个新请求。胶囊队列最多等候 16 条，排队满 45 秒或目标时间已过会跳过。已受理的即时通知由宿主处理，定时业务仍通过已有的专用后台接口注册。

灵动通知每类最多显示 3 条，合计最多显示 6 条。可见通知从上到下依次按来电、错误、警告、双按钮提醒、实时倒计时、倒计时预告、成功、普通信息排序，同类按进入顺序排列。空出位置后优先补入可显示的高优先级项。宿主只播放一通来电的声音，结束后再播放下一通。通知记录由宿主本地保存，队列中的待显示项可从「更多」查看。

## 旧灵动通知、久坐和来电提醒

固定标准包`dynamic-island`、`sit-reminder`、`phone-reminder`分别声明`notifications.island`、`reminders.sit`、`reminders.phone`。包 ID 均带`local.souldesk.widgetjs.`前缀。原实例配置与开关保留，试加载拒绝原生请求。久坐、计划来电和试听走宿主灵动渠道，关闭该渠道会抑制胶囊和来电音频。

控制卡调用`island.get()`、`sit.get()`、`phone.get()`，返回`{revision,config,runtime}`。对应`.configure({requestId,expectedRevision,config})`接受UUID请求标识和完整配置，保存先检查版本，未知结果必须用原请求重试。`runtime`包含`providerAvailable/usedMilliseconds/breakActive/delivery/lastError`，启用配置和实际递送状态分别显示。配置为：

- 灵动通知：`{enabled:boolean}`。
- 久坐：`{enabled,sitInterval,breakInterval,mouseCheckInterval}`，三个整数以分钟计，默认30/5/5，范围1..120/1..60/1..120。
- 来电：`{enabled,rules:[{preset,enabled,minuteOfDay,workdays}]}`，四种固定`preset=squidward|tea|pill|study`各一项。时刻0..1439，工作日0=周日、不重复；章鱼哥提前7秒，其他0秒。不接收音频地址、脚本、外部通知或URL。

旧灵动通知控制卡继续调用`island.demo({sample})`，八种固定演示由宿主显示，call 静音。来电卡`phone.preview({preset})`直接试听原录音，计划规则仍以实例配置为准。原录音可能包含粗话。

顶部胶囊使用独立的宿主页，挂载就绪后直接显示。窗口代次和事件标识共同约束按钮、悬停及声音状态。宿主在窗口显示后向对应页面发送事件，页面随后尝试播放。歌词跟随实际音频时刻，播放受阻可点击接听重试。

休息层仅`runtime.ready`、`sit.break.peek()`、`sit.break.close({id})`；peek含`id/remainingSeconds/sampledAt`。当前久坐提醒的确认动作创建全屏置顶休息层并聚焦。Esc 和结束休息只关闭当前休息，即使数据库读取失败也有效。被动胶囊无焦点置顶。隐藏控制卡后已启用的后台继续运行，停用或移除影响对应来源。全局隐藏期间暂停弹出及播放，退出时停止。计划槽先在 SQLite 预留，按日去重。久坐每 10 秒在内存读取一次当前鼠标坐标，长间隔或读取失败不计连续活动。真实焦点、音频、鼠标与多屏须在 Windows 验收。

## AI生成工具

`ai.generate`仅向固定标准包`local.souldesk.widgetjs.ai-mindmap|ai-mermaid|ai-translate`开放。三款默认没有连接，试加载拒绝所有`ai.*`请求；正式实例可用自己的OpenAI兼容Chat Completions服务。协议依据为[官方Chat流式接口](https://developers.openai.com/api/reference/resources/chat)，不代表已验证每家兼容服务。接口不接受任意请求头、工具、脚本或HTTP动作。

- `ai.status()`返回`{revision,configured,enabled,apiBaseUrl,model,hasKey,job}`，不返回Key。
- `ai.configure({requestId,expectedRevision,apiBaseUrl,apiKey?,model,enabled})`保存连接。地址为HTTPS或本机HTTP回环的Base URL，自动追加`/chat/completions`；不含URL用户/密码、查询、片段或完整接口路径。首次/换服务必须显式提供Key，无鉴权服务显式提供空字符串；省略Key仅保留相同地址的已有Key。密文绑定当前Windows用户及实例，普通存储不保存Key。
- `ai.clear({requestId,expectedRevision})`清除连接并取消旧任务，保留空文档版本/回执，旧请求不会重新启用密钥。保存/清除结果未知时用同一UUID和相同参数重试，不能改请求内容。配置写入与版本比较同一事务，错误保留原配置。
- `ai.begin({requestId,configRevision,input})`先事务登记请求槽，返回任务快照；只有首次登记成功才创建一个HTTP作业。最后64项UUID保留指纹/状态；在此范围内相同UUID重试或重启只读取记录，不重新发送，不同内容拒绝。用户显式重新生成使用新UUID；不会自动重试付费请求。记录无结果时返回`resultAvailable=false`，不虚构成功文本。
- `ai.poll({requestId})`和`ai.cancel({requestId})`只作用本实例请求。快照含`requestId/input/state/text/error/resultAvailable/updatedAt`；状态为`running/completed/failed/cancelled/interrupted`。`input`是原请求的严格业务输入，不含模型密钥。取消/失败保留真实部分文本。全部结果只缓存在当前进程，已保存的本地内容/历史由组件普通存储负责；终态缓存不覆盖重新打开时的已保存草稿。

三种`input`均拒绝额外字段：导图`{content,extra?}`（1..10000字、说明最多500字）；图表`{content,chartType}`（原11种英文稳定标识）；翻译`{content,firstLang,secondLang}`（原20种语言标识）。核心使用固定原提示词、`stream=true`和用户模型，只接受文本SSE，不执行工具。单帧256KiB、响应2MiB、结果512KiB、请求64KiB、网络总期限120秒；中断/服务截断/错误不标完成，末帧已收到的真实文本仍保留。HTTP固定POST和鉴权头、无重定向，错误不回显服务响应正文或Key；沿用主程序网络配置。

最多4个运行任务，单实例新任务替换旧任务；最多128个实例的进程结果缓存。新任务预留失败不影响旧任务。配置变更、会话关闭/失效、实例删除和退出取消在途；250ms存活检查覆盖服务器不发新数据的情况。鉴权/结果提交保留会话锁，配置/删除与作业更新由`native_actions`协调，迟到旧回包不得重建已删除记录。用户输入/结果最多20份且总2MiB历史，历史保留原输入及完成/取消/失败身份，密钥不进入普通存储。

AI图形仅开放SVG所需内联样式及用户手势全屏；保持脚本`self`、无`eval`、无网络/外部图片/网页嵌入/工作线程。采用固定原Markmap/Mermaid真实引擎及[官方Markmap用法](https://markmap.js.org/docs/packages--markmap-view)、[Mermaid用法](https://mermaid.js.org/config/usage.html)；模型图形配置/脚本/链接和外部资源不执行，错误保留旧图并明确未更新，旧图不能当新源码导出。PNG走`documents.save({fileName,extensions:['png'],data})`正式用户选择，不直接指定路径；翻译只声明复制，图形两包另声明文档保存。原生文件选择、实际PNG/剪贴板/全屏、真实用户模型服务与桌面效果单列手动验收，离线DOM测量桩与假密钥回环不替代实机。


## 剪切板固定能力（后台已接通，组件界面待接）

剪贴板原生权限仅供固定包`local.souldesk.widgetjs.clipboard`和`local.souldesk.widgetjs.clipboard-search`。旧`clipboard.manage`保留原行为，新包声明`clipboard.control`以及对应操作权限。历史组件使用`clipboard.history`、`clipboard.copy`、`clipboard.paste`，粘贴须同时声明复制。AI剪贴板使用`clipboard.search`、`clipboard.tools`。启动采集还需要对应历史或搜索能力和已保存的用户开关。浏览器`clipboard.read/write`是独立权限。隐藏试加载拒绝这些原生操作，正式请求继续绑定会话、实例和专用窗口。[操作对应表](../docs/19-架构审计收尾.md)

- `clipboard.status({})`（也可null）：返回revision、configRevision、kind（history/search）、captureEnabled、shortcutEnabled、pasteEnabled、shortcut、historyExpiredHours、count、listening与lastError，以及actionRevision/actionError。动作回执损坏时actionRevision为null并单列actionError，不阻断历史采集。另返回searchPlatform（google/bing/baidu，默认google）、shortcutRegistered/shortcutState/shortcutError。shortcutState为disabled/pending/registered/failed，显示真实登记结果；三个开关默认false，保存配置不冒称注册或监听成功。listening表示后台已建立观察。新AI剪切板默认Super+Alt+S，历史默认Super+Alt+V。
- `clipboard.query({keyword?,page?,size?,favoritesOnly?})`：仅历史组件，默认页1/每页10，最多100；内容及标签不区分大小写，按最近使用倒序，返回revision/total/hasMore/items。
- `clipboard.get({id})`：仅读取当前历史实例的一条完整记录，含条目revision、文本content、type=text、labels/favorite/useCount、毫秒createTime/updateTime/latestUseTime。
- `clipboard.mutate({requestId,command})`：UUID请求标识，最近64项相同请求不重复执行；相同标识更换命令拒绝。返回当前状态和replayed。command严格接受以下四种type，未知字段拒绝。
  - configure：expectedRevision为配置configRevision，并明确提供captureEnabled/shortcutEnabled/pasteEnabled/shortcut/historyExpiredHours（12/24/48），可选searchPlatform（google/bing/baidu，缺省google）；AI剪切板只允许此类修改。默认搜索平台在配置指纹中省略，兼容旧UUID回执重试。
  - edit：id、expectedItemRevision、content、labels和favorite；后台新增与该条编辑不冲突，条目已变则拒绝覆盖。
  - delete：id及expectedItemRevision。
  - clear：expectedRevision为完整历史revision，只清非收藏；即使UUID回执已轮换，旧历史版本也不能迟到清掉新记录。
- `clipboard.actionStatus({requestId})`：仅历史实例，读取该实例已登记动作的state/copy/paste/error及actionRevision；不返回正文或目标句柄。重启未定动作返回unknown，不重新执行。状态只表示复制或按键提交，不能证明目标应用收到。
- `clipboard.peek({})`（也可null）：仅AI剪切板，返回本实例当前启用代次的最近text/serial/updatedAt/configRevision。文本只在进程内保留，停用/换配置/删除/退出清空，不暗中保存为历史。

纯文本历史按Windows当前用户和实例DPAPI保护，单条最多50000字符/无零字符、1000条/4MiB；收藏不为腾容量而删除。标签最多3个、各1～10个原中文范围/英文/数字/下划线且不重复。过期按最后使用时间严格早于截止点清理，新采样与打开清理使用同一规则。损坏/未知版本、保存失败保留原数据，不把空库冒充读取成功。

后台消费者独立于网页会话/视图计时器，关闭或隐藏视图不停止已启用采集；无启用消费者时不访问系统剪贴板序号或正文。一个新变化只读取一次并按有效代次分发；系统读取不占配置/数据库锁，提交前再核对固定包、能力、实例和配置。删除与清密文同事务，退出取消迟到提交并等待已经获准的短事务，不等待外部进程迟滞的系统读取。当前检查仅用假读取器和新隔离库，真实剪贴板/窗口/快捷键/输入仍待手动。

受控复制/粘贴已接入生产clipboard.act入口及原组件界面：requestId/actionRevision/configRevision/presentationId/itemId/itemRevision/paste全部固定字段。先持久预留再执行，回执不存正文/密钥/目标句柄；64回执之外的迟到请求仍由动作版本拒绝。授权回调持锁贯穿实际复制和最终输入，正文只读本实例加密历史。复制确认后才累计使用数，写入未知不发送粘贴；部分输入和结果保存失败明确unknown且不重发。原生调用只编译，离线测试使用Fake桌面，快捷窗口和生产动作授权已接通，原组件界面及真实输入仍待适配/手动验收。

### 剪切板专用窗口与主动动作（2026-11-17）

- `clipboard.open({})`：由正式控制卡呼出本实例窗口，后台捕获原应用身份；页面不传目标句柄。历史弹窗500×400、工作区底部居中并主动聚焦；AI搜索条700×54、工作区顶部居中，不自动抢焦点。重新呼出分配新presentationId，撤销旧页面动作。隐藏控制卡不撤销已启用的后台采集或快捷键。
- `clipboard.surface({})`：仅当前弹窗返回presentationId/configRevision/kind/pasteTargetAvailable/actionRevision，不返回窗口或进程句柄。没有原目标时仅能复制，不自动向当前其他应用输入。
- `clipboard.hide({})`：只撤销并关闭当前专用窗口，不停采集；配置变更、删除、全局隐藏或退出也撤销旧窗口代次。
- `clipboard.hover({entered:boolean})`：报告顶部条悬停；焦点由原生窗口事件维护。搜索条既无焦点也无悬停后约3秒隐藏，刷新文本保留当前交互。显示协调与系统剪贴板读取使用独立线程，延迟读取不拖住隐藏计时。
- `clipboard.act({requestId,actionRevision,configRevision,presentationId,itemId,itemRevision,paste})`：仅固定历史弹窗，正文只读对应版本的私有记录；paste=true还要求显式授权且宿主已有原应用目标。授权阶段保留原生/会话锁跨实际复制与最终输入，隐藏/恢复焦点阶段释放后重新验证。超时优先读取actionStatus或原UUID重试，不生成新请求重发未知输入。
- `clipboard.search({content})`：仅有效AI搜索条，按已选原平台对完整文本做查询编码并交系统浏览器；返回submitted，只表示已交给系统处理器。三个地址固定，不接受任意URL/请求头/脚本。只有显式点击或已启用的搜索快捷键会提交，采集变化不会自动联网搜索。

历史默认Super+Alt+V，启用后开关专用弹窗；AI默认Super+Alt+S，启用后按原行为直接搜索最近采样文本。快捷键按原生ID分流，剪切板事件不同时打开主面板；同一键只保留一个有效消费者，冲突显示失败，不抢占主面板或其他应用。一次失败按配置版本缓存，不循环注册；修改或保存配置后再尝试。停用/删除约一秒内释放本实例登记，旧事件即时重验配置，退出释放全部本功能登记。释放失败的旧事件禁用并在退出再释放，不作为主面板事件处理。

专用窗口使用最小桥接权限和clipboard-popup角色，不授予任意窗口、快捷键或系统输入命令。真实窗口焦点/大小/显示层级、系统剪贴板和按键接收尚未手动验；原Vue、导图/翻译入口和标准包仍未完成，不能以这些后台接口计两款完整组件。


### 剪切板本地AI工具和重建回执（2026-11-17）

`clipboard.status`增加`lastAction`，仅含最近动作回执，重建视图可回读上次未定结果，不恢复或重发原输入。`clipboard.surface`在动作记录损坏时仍返回有效presentationId，单列actionRevision=null/actionError，历史列表保留。客户端超时沿用原UUID；读取回执不执行动作，后台刷新不覆盖脏草稿，也不清除动作异常提示。设置草稿保存打开时版本，仅本客户端成功保存后推进基线。

`clipboard.tool({kind,content})`仅AI剪切板，kind固定mindmap/translate，内容须为1至10000字；打开独立工具窗口只预填，不自动请求模型。`clipboard.settings()`打开该实例专用设置窗口；普通历史仍在原弹窗内设置。工具、设置与主显示分别使用固定角色，不能打开任意网页或其他实例。

工具内复用`ai.status/configure/clear/begin/poll/cancel`。模型凭据仍属于父实例，导图和翻译各有独立任务范围、UUID回执和文档；两者能并行，关闭或重开同类工具只取消本类任务，修改连接取消该父实例的两类任务。旧窗口、旧配置及删除后的结果不能写回。`ai.toolInfo()`只返回当前窗口的kind/content/presentationId，不返回密钥。

`ai.document.get()`及`ai.document.set({document})`仅当前固定AI工具，最多2MiB对象，按Windows用户/父实例/工具DPAPI保护；不读写父实例普通storage，也不互盖两种工具内容。未知加密版本、损坏或不属于该工具的内容拒绝覆盖。工具不获得历史采集、任意网络、系统输入或通用窗口命令；PNG只复用用户主动选择保存。
## 网页助手固定能力（assistant.web）

固定包 `local.souldesk.widgetjs.ai-assistant` 使用原生网页助手；普通卡片调用 `host.request`，专用壳使用同样的组件SDK，由宿主分流到四个专用命令。远程网页和登录窗不授予本机IPC权限，组件不能传任意网址、脚本、窗口句柄或资料目录。

- `assistant.status()`：返回 `revision/config`、`surface`、`nativeCleanupPending/nativeError`、`shortcutRegistered/shortcutError`。`surface`含展示标识、`phase`、`serial/nativeSettled`、覆盖层标识及各格的导航状态，不含地址、Cookie、页面内容或登录成功标志。
- `assistant.configure({requestId,expectedRevision,config})`：完整保存有限配置；规范UUID回执防重复，失败不覆盖。同UUID重试须保留原参数，版本冲突后不得把旧草稿静默换成新基线。
- `assistant.open/hide/retry({expectedRevision})`：显式打开、收起或重建。响应表示已接收目标状态，原生完成状态另看 `nativeSettled`；清理不确定不得提示已释放。
- 专用壳的 `assistant.overlay({expectedRevision,id,visible})`：`id`仅 `guide/settings/select0/select1/select2/select3`。展开本机菜单前等待当前展示的 `nativeSettled`，防止原生子视图盖住菜单；保存平台后再关闭覆盖层。

`config`字段为 `backgroundEnabled/shortcut/alwaysTop/pageCount/platforms/position/widthRatio/proxy/tipProgress`。格数仅1/2/4；平台为固定八个枚举且固定四个偏好；位置left/right，宽度0.2/0.3/0.4/0.5，引导进度0～3。代理为 `{mode:"system"}` 或 `{mode:"manual",protocol,host,port}`，协议仅http/https/socks4/socks5，不接凭据、URL或其他字段。后台和置顶默认false，启动不自动打开网页；只有启用后台入口才注册快捷键，普通卡片关闭不停止已开启入口。

本机壳只开放上述固定方法、`runtime.ready`和只读`settings.get`；没有通用存储、行情、AI生成、剪切板、窗口或脚本能力。各格的`phase=ended`只表示该次导航成功结束，不代表站点内容或登录成功。真实窗口/登录/代理和快捷键仍应单独验收，离线状态或DOM检查不能替代。

## AI直聘固定能力（jobs.manage）

仅固定包`local.souldesk.widgetjs.ai-job`可声明。普通控制卡通过`host.request('jobs.*',参数)`操作实例，搜索/分析/知识本机壳通过专用会话桥接；远程BOSS和职位详情不持有宿主令牌或IPC权限。标准包、原页面和知识记录已接入；编译及离线检查通过，真实登录、模型调用和原生视图仍待手动验收。

- `jobs.status`与`jobs.query`：读取当前配置、模型连接状态、任务、岗位及统计；不返回模型密钥、内部评分令牌或输入散列。
- `jobs.configure`：`requestId/expectedRevision/config`保存完整求职条件及`backgroundEnabled`，版本冲突保留草稿；模型连接使用`jobs.model.configure/clear`的独立版本，凭据由实例DPAPI保存。
- `jobs.open`：只接收固定`page`。`search/analysis`不接附加字段，`knowledge`仅接完整`context`，`detail`仅接已有职位`jobId`，不接任意网址或脚本。
- `jobs.start/rerate`：绑定`requestId/configRevision/generation/modelRevision`，仅单项重评接`jobId`；预留成功后才可能请求模型，丢失响应重试不自动重复付费请求。
- `jobs.stop/clear`：分别接当前`requestId`或`expectedGeneration`。清空须先停止当前任务，只清理当前实例。
- `jobs.surface.info/ready/layout/overlay/close`：只允许当前本机壳会话；布局为固定客户区矩形，不能执行代码或指定原生句柄。

知识窗口打开不生成内容，百科/导图须点击生成。后台默认关闭；关闭未许可的任务窗口、隐藏组件、修改模型配置或移除/升级实例会撤销旧任务许可。实际原生窗口、网络取消与登录行为仍待组件完成后的手动验收。

知识记录限定当前已授权知识窗口，context从宿主窗口账本推导，组件不能传提示词、网址或脚本。`jobs.knowledge.get({kind})`仅返回已存记录/本地问候；`generate`接收`kind/requestId/expectedRevision/modelRevision`，仅interview另接`text`。`clear`接收`kind/requestId/expectedRevision`，`stop`接收`kind/requestId`。kind仅wiki、mindmap、interview；完整历史发送，不静默截断，只有完成的模型结果进入历史；中断的partial明确保留为未完成。打开窗口、读取与程序恢复均不自动生成。

`jobs.help({topic})`只接受tutorial、deepseek、qwen、spark、tencent，系统打开固定原教程或文档，不接受任意URL。知识页允许用户手势全屏与显式复制；BOSS和详情仍为原生受限视图，组件CSP继续禁止HTTP、iframe、动态脚本及工作线程。公司图片使用WebView的HTTPS图片通道，尚未实测随应用自定义代理。


## 专用番茄窗（本轮0.5.0源码、组件0.3.0）

只有固定包`local.souldesk.widgetjs.pomodoro`同时声明`focus.pomodoro`和`focus.pomodoro.window`可用。每实例私有开关默认关闭，键为`pomodoro.window/{实例UUID}`；它不共享、不启动或复制计时。原`pomodoro.get/change`与后台提醒继续使用当前选中的私有/共享源，隐藏或关闭专用视图不停止计时，原云同步仍未实现。

| 方法 | 参数与行为 |
|---|---|
| `pomodoro.window.status` | 无参数；返回`revision/config:{enabled}/surface/nativeCleanupPending/nativeError`。surface为null或当前展示的`presentationId/edge/expanded/hidden/suspended/ready/nativeSettled/fault` |
| `pomodoro.window.configure` | `{requestId,expectedRevision,config:{enabled}}`；完整CAS保存，UUID回执最多32项；未知结果只能用原UUID及相同参数重试。关闭先保存再撤销旧展示，保存失败保留原值 |
| `pomodoro.window.open` | 无参数；已显式启用后打开或恢复此实例专用窗，用户操作可申请焦点；不会隐式启用开关 |
| `pomodoro.window.hide` | 无参数；手动隐藏专用窗，保留托盘和当前计时。移入托盘不恢复手动隐藏，点击托盘或显式打开恢复 |
| `pomodoro.window.reset` | 无参数；回到所属工作区底部中央，不改变计时源或阶段 |
| `pomodoro.window.hover` | 仅当前专用窗，`{inside:boolean}`；不接受坐标，托盘悬停由宿主独立处理 |
| `pomodoro.window.drag` | 仅当前专用窗，无参数；许可绑定展示及代次，原生执行前重验。页面不能设置位置、尺寸或读取其他窗口 |
| `pomodoro.window.menu` | 仅当前可见专用窗，无参数；显示当前源场景菜单。菜单绑定窗口、会话、源及业务版本，30秒有效且选取只消费一次；专注/暂停/等待等活动状态禁止切场景 |
| `pomodoro.window.ready` | 仅当前专用窗，无参数；确认当前会话就绪。组件普通`host.ready()`已由宿主接线，无需重复调用 |

专用窗角色为`pomodoro-window`，只允许当前计时读取/修改、上述受限窗口动作、`runtime.ready`与三个固定番茄详情页；不能配置开关、切共享源、访问一般存储、行情或系统输入。普通卡片和设置详情仍可保存私有开关。切源时无草稿的专用钟面不阻止操作，真正的编辑详情仍要求先保存关闭；切源事务使所有旧会话失效。旧菜单、令牌、包版本或选中源不能修改新来源。

固定150×150逻辑像素，支持所在工作区缩放和负坐标；移出两秒后以240毫秒三次缓出收起，保留6逻辑像素阶段进度条。拖动结束吸附最近边，同距按底、顶、左、右；完成等待展开且不自动收起。几何/显隐回执读取实际系统状态，末帧确认与焦点确认独立；发出请求不代表已显示或已释放。Windows复用已有不激活显隐/定位接口，只有用户显式打开申请焦点；真实焦点、多显示器和托盘行为仍须手动验收。

退出、升级和删除先撤内存展示许可，删除许可与实例数据同一事务。原生资源未释放保留pending/error；显示故障不会后台连续重建，需用户重开。计时业务损坏不阻止读取和关闭私有开关：卡片显示数据不可用与停用入口，不把默认空快照当作实际计时。开关草稿冲突后保留原版本，主动载入已保存设置后才使用新基线。
