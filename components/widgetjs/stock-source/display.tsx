import{formatDecimal,signOf}from'../metal/metal-model.js';
import{markets}from'./stock-model.js';
export const cn=(...values:any[])=>values.filter(Boolean).join(' ');
export const formatNumber=(value:any,places=2)=>value==null?'—':formatDecimal(value,places);
export const formatMoney=(value:string)=>formatDecimal(value,2);
export const formatPercent=(value:string)=>(signOf(value)>=0?'+':'')+formatDecimal(value,2)+'%';
export function ExchangeTag({exchange}:any){return <span className="exchange-tag" title="东方财富市场标识">{markets[exchange]||exchange}</span>;}
