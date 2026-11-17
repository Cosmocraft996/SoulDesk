import { useCallback, useEffect, useRef, useState } from 'react'
import { useWidget, useWidgetStorage, WidgetWrapper } from '@widget-js/react'
import styled, { createGlobalStyle, keyframes } from 'styled-components'
import { ArrowLeftRight, RefreshCw, Settings } from 'lucide-react'
import { useSelfSelectStockStore } from '@/store/use-self-select-stock-store'
import { useStockColorStore } from '@/store/use-stock-color-store'
import type { Stock } from '@/model/stock'
import Sortable from 'sortablejs'
import { WidgetApi } from '@widget-js/core'
import { EastMoneyStockApi } from '@/api/eastmoney-stock-api'
import { useInterval } from '@/hooks/use-interval'
import StockItem, { type StockMetricMode } from './stock-item'

const StockGlobalStyle = createGlobalStyle`
  body {
    background-color: transparent;
    color: 'var(--widget-color, inherit)';
  }
  *{
    user-select: none;
  }
`

const WidgetTitle = styled.div`
  color: var(--widget-color, inherit);
  font-size: 18px;
  font-weight: 600;
  line-height: 1;
`

const spin = keyframes`
  to {
    transform: rotate(360deg);
  }
`

const RefreshIndicator = styled(RefreshCw)`
  display: inline-block;
  width: 13px;
  height: 13px;
  margin-left: 8px;
  vertical-align: -1px;
  opacity: 0.5;
  animation: ${spin} 0.8s linear infinite;
`

const REFRESH_OPTIONS = [
  { label: '30 秒', value: 30000 },
  { label: '1 分钟', value: 60000 },
  { label: '3 分钟', value: 180000 },
  { label: '5 分钟', value: 300000 },
]

const METRIC_MODE_OPTIONS: Array<{ value: StockMetricMode; label: string }> = [
  { value: 'changePercent', label: '涨跌幅' },
  { value: 'returnRate', label: '收益率' },
  { value: 'returnAmount', label: '收益额' },
]

export default function StockWidgetView() {
  const { stocks, loadAll, saveOrder } = useSelfSelectStockStore()
  const { getColorByValue, getUpColor, getDownColor, toggleStockColor, isUpColorRed } = useStockColorStore()
  const sortableRef = useRef<HTMLDivElement>(null)
  const [showSettings, setShowSettings] = useState(false)
  const [refreshInterval, setRefreshInterval] = useState<number>(60000)
  const [localStocks, setLocalStocks] = useState<Stock[]>([])
  const [loading, setLoading] = useState(false)
  const [metricMode, setMetricMode] = useState<StockMetricMode>('changePercent')
  const [widgetTitle] = useWidgetStorage('stock-widget-title', '自选股票')

  const metricModeIndex = METRIC_MODE_OPTIONS.findIndex((option) => option.value === metricMode)
  const currentMetricLabel = METRIC_MODE_OPTIONS[metricModeIndex].label
  const nextMetricOption = METRIC_MODE_OPTIONS[(metricModeIndex + 1) % METRIC_MODE_OPTIONS.length]

  const toggleMetricMode = () => {
    setMetricMode(nextMetricOption.value)
  }

  const refreshAllStocks = useCallback(async (stockList: Stock[]) => {
    if (stockList.length === 0) return
    setLoading(true)
    try {
      const results = await Promise.allSettled(
        stockList.map((stock) => EastMoneyStockApi.getByType<Stock>(stock.code, stock.type))
      )
      const refreshed = stockList.map((original, index) => {
        const result = results[index]
        if (result.status === 'fulfilled' && result.value) {
          return {
            ...original,
            ...result.value,
            sortOrder: original.sortOrder,
            holdingPrice: original.holdingPrice,
            holdingShares: original.holdingShares,
          }
        }
        return original
      })
      setLocalStocks(refreshed)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadAll()
  }, [loadAll])

  useWidget()

  useEffect(() => {
    if (stocks.length > 0) {
      setLocalStocks(stocks)
      refreshAllStocks(stocks)
    } else {
      setLocalStocks([])
    }
  }, [stocks, refreshAllStocks])

  useInterval(() => {
    if (localStocks.length > 0) {
      refreshAllStocks(localStocks)
    }
  }, refreshInterval)

  useEffect(() => {
    if (sortableRef.current && localStocks.length > 0) {
      const sortable = Sortable.create(sortableRef.current, {
        animation: 200,
        handle: '.drag-handle',
        onEnd: async (evt) => {
          if (evt.oldIndex !== undefined && evt.newIndex !== undefined && evt.oldIndex !== evt.newIndex) {
            const newList = [...localStocks]
            const [removed] = newList.splice(evt.oldIndex, 1)
            newList.splice(evt.newIndex, 0, removed)
            setLocalStocks(newList)
            await saveOrder(newList)
          }
        },
      })
      return () => sortable.destroy()
    }
  }, [localStocks, saveOrder])


  return (
    <WidgetWrapper>
      <StockGlobalStyle />
      <div
        className="stock-widget-root w-full h-full text-white flex flex-col rounded-2xl overflow-hidden pt-2"
      >
        <div className="flex items-center justify-between px-4 pt-3 pb-1.5">
          <WidgetTitle>
            {widgetTitle}
            {loading && <RefreshIndicator aria-label="刷新中" />}
          </WidgetTitle>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="opacity-40 hover:opacity-100 transition-all rounded-lg hover:bg-white/10"
              title={`当前显示${currentMetricLabel}，点击切换为${nextMetricOption.label}`}
              aria-label={`将${currentMetricLabel}切换为${nextMetricOption.label}`}
              onClick={toggleMetricMode}
            >
              <ArrowLeftRight className="h-5 w-5 opacity-80" style={{ color: 'var(--widget-color, inherit)' }} />
            </button>
            <button
                type="button"
                className="opacity-40 hover:opacity-100 transition-all rounded-lg hover:bg-white/10"
                title="设置"
                onClick={() => WidgetApi.openConfigPage()}
              >
                <Settings className="h-5 w-5 opacity-80" style={{ color: 'var(--widget-color, inherit)' }} />
            </button>
          </div>
        </div>

        <div ref={sortableRef} className="flex-1 overflow-y-auto px-2 pb-3 space-y-1.5 mt-1">
          {localStocks.map((stock, idx) => (
            <StockItem
              key={stock.code}
              stock={stock}
              idx={idx}
              isLast={idx === localStocks.length - 1}
              metricMode={metricMode}
              getColorByValue={getColorByValue}
            />
          ))}

          {localStocks.length === 0 && !loading && (
            <div className="text-center py-12 opacity-50 text-sm">
              暂无自选股票
            </div>
          )}

          {localStocks.length === 0 && loading && (
            <div className="text-center py-12 opacity-50 text-sm">
              加载中...
            </div>
          )}
        </div>
      </div>
    </WidgetWrapper>
  )
}
