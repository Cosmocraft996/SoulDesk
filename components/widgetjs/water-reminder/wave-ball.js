// 转译自固定提交的WaveBall.ts，保留双层正弦水波参数及圆形裁切。
export class WaveBall{
  constructor(options={}){this.size=Number.isFinite(options.size)&&options.size>0?Math.ceil(options.size):200;this.x=options.x??this.size/2;this.y=options.y??this.size/2;this.progress=options.progress??0;this.color=options.color??'rgba(55,133,207,.75)';this.waveWidth=.025;this.waveHeight=5.6;this.offsetX=[0,0];this.startX=0;this.vx=[.05,.01];}
  render(ctx){this.ctx=ctx;this.bgColor=ctx.createLinearGradient(this.size/2,this.size/2,this.size/2,this.size);this.bgColor.addColorStop(0,this.color);this.bgColor.addColorStop(1,this.color);return this;}
  drawWave(n){this.offsetX[n]+=this.vx[n];const {ctx,size,x,y}=this;const height=-this.progress/100*size;ctx.save();ctx.translate(x,y);ctx.beginPath();ctx.arc(0,0,size/2,0,2*Math.PI);ctx.clip();ctx.beginPath();for(let i=-size/2;i<size/2;i++){const h=this.waveHeight*Math.sin((this.startX+i)*this.waveWidth+this.offsetX[n]);ctx.lineTo(i,h+size/2+height);}ctx.lineTo(size/2,size/2);ctx.lineTo(-size/2,size/2);ctx.fillStyle=this.bgColor;ctx.fill();ctx.restore();}
  update(progress){if(progress!==undefined)this.progress=progress;this.drawWave(0);this.drawWave(1);}
}
