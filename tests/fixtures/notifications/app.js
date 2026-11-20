import { connectHost } from './host.js';

const host = await connectHost();
const samples = document.getElementById('samples');
const status = document.getElementById('status');
const channelInput = document.getElementById('channel');
const bodies = {
  info: '这是一条来自测试组件的普通消息。',
  success: '测试任务已完成。',
  warning: '这是一条警告示例。',
  error: '这是一条错误样式示例。',
  reminder: '点「打开组件」回到这张测试卡。',
  'advance-countdown': '点胶囊切换到实时倒计时。',
};

samples.addEventListener('click', async event => {
  const button = event.target.closest('button[data-kind]');
  if (host.preview || !button || samples.disabled) return;
  const kind = button.dataset.kind;
  const label = button.textContent.trim();
  const requestId = crypto.randomUUID();
  const channel = channelInput.value;
  const payload = kind === 'call'
    ? { requestId, channel, kind, preset: 'study' }
    : { requestId, channel, kind, title: label };
  if (bodies[kind]) payload.body = bodies[kind];
  if (kind === 'advance-countdown' || kind === 'countdown') {
    payload.targetAt = Date.now() + (kind === 'advance-countdown' ? 25_000 : 15_000);
  }

  samples.disabled = true;
  status.dataset.state = '';
  status.textContent = `正在提交「${label}」…`;
  try {
    const reply = await host.request('notifications.send', payload);
    if (reply.status === 'suppressed') {
      status.dataset.state = 'warning';
      status.textContent = `「${label}」被宿主抑制。检查所选渠道、隐藏全部和模拟来电开关。`;
    } else if (reply.status === 'accepted') {
      const channel = reply.channel === 'island' ? '灵动通知' : 'Windows 系统通知';
      status.textContent = `已受理「${label}」 · ${channel}。连续发送时可能排队。`;
    } else {
      status.dataset.state = 'warning';
      status.textContent = '宿主回执格式异常，请确认运行的是随附的验收版本。';
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    status.dataset.state = 'error';
    status.textContent = message.includes('过于频繁')
      ? '同一实例每 30 秒最多 5 次。等前一批超过 30 秒后再试。'
      : message;
  } finally {
    samples.disabled = false;
  }
});

await host.ready();
samples.disabled = host.preview;
status.textContent = host.preview ? '试加载期间只显示界面。' : '选择渠道后逐项点击，观察对应通知。';
