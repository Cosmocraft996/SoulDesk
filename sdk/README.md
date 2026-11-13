# SoulDesk 普通组件接口 v1（G2）

组件为独立静态 ZIP 包，根目录放 `manifest.json`，资源放 `dist/`。`examples/template/` 是最小模板，`examples/clock/` 是时钟示例；执行 `pnpm example:build` 独立生成两个外部包。当前源码也将卡片、时钟和行情作为预置组件编译到主程序，首次添加仍使用同一校验、试加载和消息桥；不会自动创建实例或覆盖已导入版本。指定源码目录时可执行 `node scripts/build-example.mjs examples/template`。

已实现 `settings.get`、`settings.update`、`storage.get` 和 `storage.set`，均要求 `storage.instance`。`host.js` 完成来源与父窗口核对、消息通道握手、请求编号和 10 秒超时；组件不直接调用 Tauri。

```javascript
import { connectHost } from './host.js';
const host = await connectHost();
const settings = await host.request('settings.get');
await host.request('settings.update', { ...settings, title: '我的时钟' });
// 初始化界面成功后报告就绪；初始化异常应继续抛出，让宿主阻止失败包启用。
await host.ready();
```

宿主按会话绑定实例和容器；组件不能通过参数指定其他实例。移动、重新加载和销毁使旧会话失效。`settings.update` 替换完整配置，成功返回值表示 SQLite 已保存，失败保留旧配置；`storage.set` 同样替换当前实例的存储对象。

配置结构支持 object、properties、additionalProperties=false、required，以及 boolean/string/number/integer 基本属性；属性支持 default、title、minimum、maximum、maxLength。配置结构、配置与存储对象各限 16 KiB，属性最多 64 项。minimum/maximum 仅用于数值，maxLength 仅用于字符串；约束须有正确类型且范围不能倒置。

默认值由宿主显式生成，导入时必须能产生有效的新实例配置；必填项需要合法默认值，不能引用未知或重复属性。这个约定比 JSON Schema 自身的默认值注解更严格。integer 接受 `1.0` 这样的整数值，限于 JavaScript 安全整数范围 ±9007199254740991；普通配置数值不用于金融价格。

包限制：压缩大小 20 MiB，展开大小 50 MiB，最多 256 项。宿主拒绝越界、绝对路径、符号链接、Windows 保留名及不区分大小写的重名路径；只登记已完整验证的资源目录。

导入顺序：校验 → 不可变版本目录 → 隐藏试加载 → 事务启用。相同内容重复导入幂等，不撤销现有会话。升级仅接受更高数字版本；同版本的不同内容及降级明确拒绝。新版须兼容所有旧实例配置，只补充缺少的默认项，不静默重置。试加载分别使用默认配置及已有实例配置/存储的内存副本，只能写隔离数据；并发配置变化会中止本次切换。

新组件清单添加 `"loadCheck": 1`，并在初始化完成后 `await host.ready()`。宿主等待就绪且至少 800 ms 无捕获到的脚本/资源错误，10 秒内未就绪则保留原版。未声明该字段的 G0 草案包保留旧的成功接口请求判定。试加载不能证明未来任意时刻都不会出错；不隔离死循环，也不能识别被组件自己吞掉且谎报成功的错误。

启用时保存切换前的实例配置/存储快照并保留旧版本文件；事务失败不改变原登记。新会话绑定不可变资源摘要，旧页面不能在升级后取得新版会话。取消组件登记须先移除其实例，历史版本文件不会自动删除。

页面资源采用相对路径，脚本必须为外部文件。允许 HTML/CSS/JS/MJS/JSON、常见图片、字体、TXT/MAP；拒绝系统脚本、动态库和可执行文件。`dist/__souldesk_runtime.js` 是宿主保留名。禁止内联脚本、网络连接、嵌套框架、弹窗、表单导航和宿主命令。宿主保留文档声明并注入错误监测脚本。组件是用户信任的本地代码，容器不能承诺隔离页面死循环或网页进程崩溃。

Windows 默认组件来源为 `http://widget.localhost`；SDK 从宿主提供的入口 URL 取得准确的宿主来源，不向通配符发送有权限消息。实际来源行为需结合安装版验收记录理解。

开发流程：复制模板并修改稳定 ID → 定义配置及默认值 → 用相对资源和 SDK 实现界面 → 初始化后报告就绪 → 独立构建 ZIP → 从工作台导入 → 新建两个实例并核对配置隔离。普通组件不用修改或重新构建主程序；需要新的特殊后台能力时再升级宿主。

## 行情与警报能力（宿主0.3.0起）

`examples/market/` 使用同一协议独立构建：`node scripts/build-example.mjs examples/market`，当前产物 `artifacts/local.souldesk.market-0.1.1.zip`。同一源码也供预置行情组件使用；外部包仍可独立分发和升级，旧宿主遇到未知能力会拒绝导入。

| 清单能力 | 接口 |
|---|---|
| `market.read` | `market.catalog`、`watchlist.get/set`、`market.subscribe/unsubscribe/poll` |
| `alerts.manage` | `alerts.list/save/delete/history/retry` |

行情键为 `{exchange:'binance',market:'usd_m',symbol:'BTCUSDT',priceKind:'last'|'mark'}`。价格、阈值和步长均为字符串。`watchlist.set` 替换本实例有序列表，最多20项，后台持续保留Last/Mark消费者；规则是独立宿主对象，不随实例删除。

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

`alerts.save` 参数为 `{rule,expectedRevision}`。新建revision=1、expectedRevision=null；编辑传旧版本号，并把revision加1。过期编辑明确冲突。`alerts.list` 返回 `{rule,state}` 列表；保存前必须提示首次启用或业务条件修改后已满足会提醒一次。仅改名称或通知策略不重新武装。

`alerts.history` 使用offset/limit（最多100），返回逻辑事件、两个独立渠道的状态/尝试次数/错误。`alerts.retry` 接收同一事件id及channel=`notification`或`sound`，仅对failed/unknown/expired执行，每渠道累计最多3次。sent不等于用户实际看到；不确定结果不得显示为失败后自动重播。

隐藏试加载只接收隔离配置和存储。组件应在 `host.preview` 为true时跳过真实行情/警报，先完成静态界面初始化，再 `await host.ready()`。本地表单的提交事件已允许；内容安全策略仍禁止表单导航和外部网络。确认交互使用HTML `dialog`；容器不开放原生alert/confirm/prompt。
