import { chartGeometry } from './gold-model.js';
// 保留原画布的金色折线、渐隐填充、最新价虚线和端点；仅投影坐标使用浮点。
export function drawGoldChart(canvas,series,current){
  const context=canvas.getContext('2d');if(!context)return;
  const rect=canvas.getBoundingClientRect(),ratio=devicePixelRatio||1;
  canvas.width=Math.max(1,Math.round(rect.width*ratio));canvas.height=Math.max(1,Math.round(rect.height*ratio));context.scale(ratio,ratio);context.clearRect(0,0,rect.width,rect.height);
  if(!series.length||!current||rect.width<=0||rect.height<=8)return;
  const {points,latestY}=chartGeometry(series,current,rect.width,rect.height),color='#ffc85b';
  context.beginPath();points.forEach((point,index)=>index?context.lineTo(point.x,point.y):context.moveTo(point.x,point.y));context.strokeStyle=color;context.lineWidth=1;context.lineJoin='round';context.lineCap='round';context.stroke();
  context.lineTo(rect.width,rect.height-4);context.lineTo(0,rect.height-4);context.closePath();
  const gradient=context.createLinearGradient(0,4,0,rect.height-4);gradient.addColorStop(0,color);gradient.addColorStop(1,'rgba(255,200,91,0)');context.fillStyle=gradient;context.globalAlpha=.35;context.fill();context.globalAlpha=1;
  context.save();context.setLineDash([4,4]);context.strokeStyle=color;context.globalAlpha=.8;context.beginPath();context.moveTo(0,latestY);context.lineTo(rect.width,latestY);context.stroke();context.restore();
  context.beginPath();context.arc(Math.max(0,rect.width-1),latestY,3,0,Math.PI*2);context.fillStyle=color;context.fill();
}
