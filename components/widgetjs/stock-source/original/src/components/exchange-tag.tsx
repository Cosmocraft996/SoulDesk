import * as React from 'react'
import { cn } from '@/lib/utils'

interface ExchangeTagProps {
  exchange: string
  className?: string
  style?: React.CSSProperties
  size?: 'xs' | 'sm' | 'md'
}

const flagConfig: Record<string, { country: string; title: string }> = {
  cn: { country: 'cn', title: '中国A股' },
  sh: { country: 'cn', title: '上海证券交易所' },
  sz: { country: 'cn', title: '深圳证券交易所' },
  bj: { country: 'cn', title: '北京证券交易所' },
  hk: { country: 'hk', title: '香港联合交易所' },
  us: { country: 'us', title: '美国股市' },
  nyse: { country: 'us', title: '纽约证券交易所' },
  nasdaq: { country: 'us', title: '纳斯达克' },
  amex: { country: 'us', title: '美国证券交易所' },
  jp: { country: 'jp', title: '日本股市' },
  t: { country: 'jp', title: '东京证券交易所' },
  kr: { country: 'kr', title: '韩国股市' },
  kq: { country: 'kr', title: '科斯达克' },
  uk: { country: 'gb', title: '英国股市' },
  gb: { country: 'gb', title: '英国股市' },
  l: { country: 'gb', title: '伦敦证券交易所' },
  de: { country: 'de', title: '德国股市' },
  fr: { country: 'fr', title: '法国股市' },
  sg: { country: 'sg', title: '新加坡股市' },
  au: { country: 'au', title: '澳大利亚股市' },
  ca: { country: 'ca', title: '加拿大股市' },
}

const sizes: Record<NonNullable<ExchangeTagProps['size']>, { w: string; h: string }> = {
  xs: { w: 'w-[18px]', h: 'h-[13.5px]' },
  sm: { w: 'w-5', h: 'h-[15px]' },
  md: { w: 'w-6', h: 'h-[18px]' },
}

export function ExchangeTag({ exchange, className, style, size = 'sm' }: ExchangeTagProps) {
  const key = exchange?.toLowerCase() || ''
  const config = flagConfig[key]

  if (config) {
    const s = sizes[size]
    return (
      <img
        src={`https://flagcdn.com/w40/${config.country}.png`}
        alt={config.title}
        title={config.title}
        loading="lazy"
        style={style}
        className={cn(
          s.w,
          s.h,
          'object-cover shrink-0 rounded-[2px] border border-white/10 align-middle',
          className
        )}
        onError={(e) => {
          const target = e.currentTarget
          target.onerror = null
          target.src = `https://flagcdn.com/w40/${config.country}.webp`
        }}
      />
    )
  }

  return (
    <span
      title={exchange || ''}
      style={style}
      className={cn(
        'px-1.5 py-0 text-[0.7rem] rounded border border-border bg-card text-muted-foreground',
        className
      )}
    >
      {exchange?.toUpperCase() || ''}
    </span>
  )
}
