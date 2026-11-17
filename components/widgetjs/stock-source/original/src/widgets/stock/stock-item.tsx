import { useMemo } from 'react'
import styled from 'styled-components'
import { ExchangeTag } from '@/components/exchange-tag'
import { cn, formatMoney, formatNumber, formatPercent } from '@/lib/utils'
import type { Stock } from '@/model/stock'

const RatioBadge = styled.span<{ $backgroundColor: string }>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 64px;
  height: 24px;
  border-radius: 6px;
  color: white;
  background-color: ${({ $backgroundColor }) => $backgroundColor};
  font-size: 16px;
  font-weight: 400;
  font-variant-numeric: tabular-nums;
`

interface MiniSparklineProps {
  ratio: number
  color: string
  seed: string
}

function MiniSparkline({ ratio, color, seed }: MiniSparklineProps) {
  const isUp = ratio >= 0

  const points = useMemo(() => {
    let h = 0
    for (let i = 0; i < seed.length; i++) {
      h = Math.imul(31, h) + seed.charCodeAt(i) | 0
    }
    const rng = () => {
      h = Math.imul(h ^ h >>> 16, 0x85ebca6b) | 0
      h = Math.imul(h ^ h >>> 13, 0xc2b2ae35) | 0
      const r = (h ^= h >>> 16) >>> 0
      return r / 4294967296
    }
    const n = 24
    const startValue = 0.5
    let v0 = Math.abs(ratio) / 5 + 0.02
    const pts: number[] = []
    let cur = startValue
    for (let i = 0; i < n; i++) {
      const t = i / (n - 1)
      const trend = isUp ? -v0 + 2 * v0 * t + v0 * Math.sin(t * Math.PI * 1.2) : v0 - 2 * v0 * t
      cur = startValue + trend + (rng() - 0.5) * 0.12
      pts.push(Math.max(0.05, Math.min(0.95, cur)))
    }
    return pts
  }, [ratio, isUp, seed])

  const W = 60
  const H = 32
  const padX = 4
  const padY = 4
  const graphW = W - padX * 2
  const graphH = H - padY * 2

  const { pathD, areaD, startY } = useMemo(() => {
    const max = Math.max(...points)
    const min = Math.min(...points)
    const range = max - min || 1
    const startVal = 0.5
    const pathArr = points.map((v, i) => {
      const x = padX + (i / (points.length - 1)) * graphW
      const y = padY + graphH * (1 - (v - min) / range)
      return { x, y }
    })
    const sy = padY + graphH * (1 - (startVal - min) / range)
    const d = pathArr.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x.toFixed(2)},${p.y.toFixed(2)}`).join(' ')
    const a = `${d} L${(padX + graphW).toFixed(2)},${(padY + graphH).toFixed(2)} L${padX.toFixed(2)},${(padY + graphH).toFixed(2)} Z`
    return { pathD: d, areaD: a, startY: sy }
  }, [points, graphW, graphH, padX, padY])

  const gradId = useMemo(() => `spark-${Math.random().toString(36).slice(2, 9)}`, [])

  return (
    <svg
      width={W}
      height={H}
      viewBox={`0 0 ${W} ${H}`}
      style={{ flexShrink: 0 }}
      preserveAspectRatio="none"
    >
      <defs>
        <linearGradient id={gradId} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.35" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <line
        x1={padX}
        x2={padX + graphW}
        y1={startY}
        y2={startY}
        stroke={color}
        strokeOpacity="0.55"
        strokeWidth="1.2"
        strokeDasharray="3 3"
      />
      <path d={areaD} fill={`url(#${gradId})`} />
      <path
        d={pathD}
        fill="none"
        stroke={color}
        strokeWidth="1.8"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  )
}

interface StockItemProps {
  stock: Stock
  idx: number
  isLast: boolean
  metricMode: StockMetricMode
  getColorByValue: (value: number) => { color: string }
}

export type StockMetricMode = 'changePercent' | 'returnRate' | 'returnAmount'

export default function StockItem({ stock, idx, isLast, metricMode, getColorByValue }: StockItemProps) {
  const ratio = Number.parseFloat(stock.ratio) || 0
  const increase = Number.parseFloat(stock.increase) || 0
  const price = Number.parseFloat(stock.price) || 0
  const holdingPrice = stock.holdingPrice || 0
  const holdingShares = stock.holdingShares || 0
  const returnRate = holdingPrice > 0 ? ((price - holdingPrice) / holdingPrice) * 100 : 0
  const returnAmount = holdingPrice > 0 && holdingShares > 0
    ? (price - holdingPrice) * holdingShares
    : 0
  const metricValue = metricMode === 'changePercent'
    ? ratio
    : metricMode === 'returnRate'
      ? returnRate
      : returnAmount
  const metricText = metricMode === 'changePercent'
    ? formatPercent(ratio)
    : metricMode === 'returnRate'
      ? formatPercent(returnRate)
      : `${returnAmount >= 0 ? '+' : ''}${formatMoney(returnAmount)}`
  const { color: marketColor } = getColorByValue(ratio)
  const { color: metricColor } = getColorByValue(metricValue)

  return (
    <div
      key={stock.code}
      className={cn(
        'drag-handle grid grid-cols-[100px_60px_minmax(52px,1fr)_64px] items-center gap-x-2 py-2 px-2 rounded-xl cursor-grab active:cursor-grabbing transition-colors hover:bg-white/5',
        !isLast && ''
      )}
    >
      <div className="flex min-w-0 flex-col justify-center gap-1">
        <span className="text-[16px] font-semibold leading-[1] truncate text-white" style={{ color: 'var(--widget-color, inherit)' }}>
          {stock.name}
        </span>
        <div className="flex items-center gap-1.5 mt-1">
          <ExchangeTag
            exchange={stock.market || 'us'}
            size="xs"
            style={{ height: '10px', width: 'auto' }}
            className="!w-auto"
          />
          <span className="text-[12px] leading-none tracking-wide" style={{ color: 'var(--widget-color, inherit)' }}>
            {stock.code}
          </span>
        </div>
      </div>

      <div className="flex justify-center">
        <MiniSparkline ratio={ratio} color={marketColor} seed={stock.code} />
      </div>

      <div className="flex min-w-0 flex-col items-end justify-center gap-1.5">
        <span className="text-[12px] font-semibold tabular-nums leading-none" style={{ color: 'var(--widget-color, inherit)' }}>
          {formatNumber(stock.price, 3)}
        </span>
        <span
          className="text-[12px] font-semibold tabular-nums leading-none"
          style={{ color: marketColor }}
        >
          {increase >= 0 ? '+' : ''}
          {formatNumber(String(increase), 2)}
        </span>
      </div>

      <div className="flex justify-end">
        <RatioBadge $backgroundColor={metricColor} title={metricText}>
          {metricText}
        </RatioBadge>
      </div>
    </div>
  )
}
