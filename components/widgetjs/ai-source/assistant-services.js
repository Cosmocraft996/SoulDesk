// 名称、顺序与图标取自固定原AiUtils；网址只由原生宿主使用。
import ChatGpt from'./original/src/assets/chatgpt-icon.png';
import Grok from'./original/src/assets/grok.png';
import Gemini from'./original/src/assets/google-gemini-icon.png';
import DeepSeek from'./original/src/assets/deepseek.png';
import Kimi from'./original/src/assets/kimi.png';
import Qwen from'./original/src/assets/qwen.png';
import DouBao from'./original/src/assets/dou_bao.png';
import YuanBao from'./original/src/assets/yuanbao.png';
const service=(value,name,logo)=>({value,name,logo});
const usAiServices=[service('chatgpt','ChatGPT',ChatGpt),service('grok','Grok',Grok),service('gemini','Gemini',Gemini)];
const chinaAiServices=[service('deepseek','DeepSeek',DeepSeek),service('kimi','kimi',Kimi),service('qwen','通义千问',Qwen),service('dou_bao','豆包',DouBao),service('yuan_bao','元宝',YuanBao)];
const list=[...usAiServices,...chinaAiServices];
export const AiUtils={usAiServices,chinaAiServices,aiServices:list,get:value=>typeof value==='number'?list[value]??list[3]:list.find(p=>p.value===value)??list[3]};
export const labels={configTitle:'组件设置',shortcut:'呼出快捷键',position:'窗口位置',left:'左侧',right:'右侧',windowSize:'窗口大小',percent20:'20%',percent30:'30%',percent40:'40%',percent50:'50%',aiAssistant:'AI 助手',singlePage:'单页布局',twoLine:'双行布局',fourGrid:'四格布局',setting:'设置',alwaysTop:'置顶',know:'知道了',twoLineTitle:'设置双行布局',twoLineDesc:'双行布局可以同时使用两个AI服务，对比效果',selectAiTitle:'选择AI平台',selectAiDesc:'在每个分屏底部选择要使用的AI平台',selectAiPlaceholder:'选择AI平台',chinaAi:'国内平台',usAi:'海外平台',tipTitle:'使用说明',tipShortcut:'后台入口启用后使用快捷键',tipAbroad:'各平台由你手动登录，按其实际账号、地区与网络条件提供服务。',tipRegion:'需要时，可在设置中填写自己的代理；后台入口和置顶默认关闭。',tipSetting:'设置页面'};
