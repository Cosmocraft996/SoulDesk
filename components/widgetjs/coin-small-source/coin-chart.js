import { chartGeometry } from '../gold-source/gold-model.js';

export function drawCoinChart(canvas, values, color, ratio = globalThis.devicePixelRatio || 1) {
  const context = canvas.getContext('2d');
  if (!context) return;
  const rect = canvas.getBoundingClientRect();
  canvas.width = Math.max(1, rect.width * ratio);
  canvas.height = Math.max(1, rect.height * ratio);
  context.scale(ratio, ratio);
  context.clearRect(0, 0, rect.width, rect.height);
  if (!values.length || rect.height <= 8 || rect.width <= 0) return;
  const { points } = chartGeometry(values, values.at(-1), rect.width, rect.height);
  // 单个采样没有线段，填充也会退化；直接标记该真实采样，不补造历史。
  if (points.length === 1) {
    context.beginPath();
    context.arc(points[0].x, points[0].y, 2, 0, Math.PI * 2);
    context.fillStyle = color;
    context.fill();
    return;
  }
  context.beginPath();
  points.forEach((point, index) => index ? context.lineTo(point.x, point.y) : context.moveTo(point.x, point.y));
  context.strokeStyle = color;
  context.lineWidth = 1.5;
  context.lineJoin = 'round';
  context.lineCap = 'round';
  context.stroke();
  const gradient = context.createLinearGradient(0, 4, 0, rect.height - 4);
  gradient.addColorStop(0, color + '3d');
  gradient.addColorStop(1, color + '00');
  context.lineTo(rect.width, rect.height - 4);
  context.lineTo(0, rect.height - 4);
  context.closePath();
  context.fillStyle = gradient;
  context.fill();
}
