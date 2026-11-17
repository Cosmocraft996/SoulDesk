import type { WidgetThemeOption } from '@widget-js/react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import type { ColumnDef } from '@tanstack/react-table'
import { useWidgetStorage, useWidgetTheme, WidgetThemeForm, Window } from '@widget-js/react'
import { useSelfSelectStockStore } from '@/store/use-self-select-stock-store'
import { useStockColorStore } from '@/store/use-stock-color-store'
import { EastMoneyStockApi } from '@/api/eastmoney-stock-api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { DataTable } from '@/components/ui/data-table'
import type { DataTableFeatures } from '@/components/ui/data-table-features'
import { ExchangeTag } from '@/components/exchange-tag'
import { Separator } from '@/components/ui/separator'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { ArrowDown, ArrowUp, ChevronDown, ChevronUp, Loader2, RefreshCw, Trash2 } from 'lucide-react'
import { StockSelect } from '@/components/stock-select'
import { cn, formatNumber } from '@/lib/utils'
import type { Stock } from '@/model/stock'

const stockThemeOption: WidgetThemeOption = {
  useGlobalTheme: true,
  borderRadius: true,
  backgroundColor: true,
  backgroundBorderColor: true,
  primaryColor: true,
  color: true,
} as const

const REFRESH_OPTIONS = [
  { label: '30 秒', value: 30000 },
  { label: '1 分钟', value: 60000 },
  { label: '3 分钟', value: 180000 },
  { label: '5 分钟', value: 300000 },
  { label: '10 分钟', value: 600000 },
]

interface StockConfigViewProps {
  open?: boolean
  onOpenChange?: (open: boolean) => void
  onSave?: (closeWindow?: boolean) => void
}

export default function StockConfigView({ open = true, onOpenChange, onSave }: StockConfigViewProps) {
  const { stocks, loadAll, saveStock, saveOrder, deleteStock } = useSelfSelectStockStore()
  const { stockColor, setUpColorRed, setUpColorGreen } = useStockColorStore()
  const { widgetTheme, setWidgetTheme } = useWidgetTheme()

  const [widgetTitle, setWidgetTitle] = useWidgetStorage('stock-widget-title', '自选股票')
  const [refreshInterval, setRefreshInterval] = useState('60000')
  const [localStocks, setLocalStocks] = useState<Stock[]>([])
  const [listLoading, setListLoading] = useState(false)

  useEffect(() => {
    loadAll()
  }, [loadAll])

  const refreshAllStocks = useCallback(async (stockList: Stock[]) => {
    if (stockList.length === 0) return
    setListLoading(true)
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
      setListLoading(false)
    }
  }, [])

  useEffect(() => {
    if (stocks.length > 0) {
      setLocalStocks(stocks)
      refreshAllStocks(stocks)
    } else {
      setLocalStocks([])
    }
  }, [stocks, refreshAllStocks])

  const handleAddStock = async (stock: Stock) => {
    if (!localStocks.find((s) => s.code === stock.code)) {
      setLocalStocks([...localStocks, stock])
      await saveStock(stock)
    }
  }

  const handleDeleteStock = async (code: string) => {
    setLocalStocks(localStocks.filter((s) => s.code !== code))
    await deleteStock(code)
  }

  const handleMoveStock = async (code: string, direction: -1 | 1) => {
    const currentIndex = localStocks.findIndex((stock) => stock.code === code)
    const targetIndex = currentIndex + direction
    if (currentIndex < 0 || targetIndex < 0 || targetIndex >= localStocks.length) return

    const reordered = [...localStocks]
    const [movedStock] = reordered.splice(currentIndex, 1)
    reordered.splice(targetIndex, 0, movedStock)
    const stocksWithOrder = reordered.map((stock, index) => ({ ...stock, sortOrder: index }))

    setLocalStocks(stocksWithOrder)
    await saveOrder(stocksWithOrder)
  }

  const updateHoldingField = (code: string, field: 'holdingShares' | 'holdingPrice', value: string) => {
    setLocalStocks((prev) =>
      prev.map((s) => (s.code === code ? { ...s, [field]: Number.parseFloat(value) || 0 } : s))
    )
  }

  const stockColumns = useMemo<ColumnDef<DataTableFeatures, Stock, any>[]>(() => [
    {
      id: 'stock',
      header: () => <span className="font-semibold text-base">股票</span>,
      cell: ({ row }: any) => {
        const stock = row.original as Stock
        return (
          <div className="flex items-center gap-2 min-w-0">
            <ExchangeTag
              exchange={stock.exchange}
              style={{ width: 18, height: 12 }}
            />
            <span className="font-medium truncate">{stock.name}</span>
            <span className="text-xs opacity-60 font-mono shrink-0">{stock.code}</span>
          </div>
        )
      },
    },
    {
      id: 'price',
      header: () => <div className="text-right font-semibold text-base">现价</div>,
      cell: ({ row }: any) => (
        <div className="text-right font-semibold tabular-nums">
          {formatNumber((row.original as Stock).price)}
        </div>
      ),
    },
    {
      id: 'holdingShares',
      meta: { width: 120 },
      header: () => <div className="text-right font-semibold text-base">持有份额</div>,
      cell: ({ row }: any) => {
        const stock = row.original as Stock
        return (
          <Input
            type="number"
            min={0}
            step="0.01"
            className="h-8 w-full text-sm tabular-nums"
            value={stock.holdingShares ?? 0}
            onChange={(e) => updateHoldingField(stock.code, 'holdingShares', e.target.value)}
          />
        )
      },
    },
    {
      id: 'holdingPrice',
      meta: { width: 120 },
      header: () => <div className="text-right font-semibold text-base">持仓价</div>,
      cell: ({ row }: any) => {
        const stock = row.original as Stock
        return (
          <Input
            type="number"
            min={0}
            step="0.01"
            className="h-8 w-full text-sm tabular-nums"
            value={stock.holdingPrice ?? 0}
            onChange={(e) => updateHoldingField(stock.code, 'holdingPrice', e.target.value)}
          />
        )
      },
    },
    {
      id: 'actions',
      header: () => <div className="text-center font-semibold text-base">操作</div>,
      cell: ({ row }: any) => {
        const stock = row.original as Stock
        const stockIndex = localStocks.findIndex((item) => item.code === stock.code)
        return (
          <div className="flex items-center justify-center gap-0.5">
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              disabled={stockIndex <= 0}
              title="上移"
              aria-label={`上移 ${stock.name}`}
              onClick={() => handleMoveStock(stock.code, -1)}
            >
              <ChevronUp className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              disabled={stockIndex < 0 || stockIndex >= localStocks.length - 1}
              title="下移"
              aria-label={`下移 ${stock.name}`}
              onClick={() => handleMoveStock(stock.code, 1)}
            >
              <ChevronDown className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 text-destructive hover:text-destructive"
              title="删除"
              aria-label={`删除 ${stock.name}`}
              onClick={() => handleDeleteStock(stock.code)}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        )
      },
    },
  ], [localStocks])

  const footer = (
    <div className="flex flex-wrap justify-end gap-3">
      <Button type="button" onClick={() => window.close()}>
        关闭
      </Button>
    </div>
  )

  if (!open) return null

  return (
    <Window title="股票设置" footer={footer}>
      <Tabs defaultValue="content" className="gap-4">
        <TabsList className="w-full justify-start">
          <TabsTrigger value="content">组件设置</TabsTrigger>
          <TabsTrigger value="theme">主题设置</TabsTrigger>
        </TabsList>

        <TabsContent
          value="content"
          className="rounded-xl border bg-card p-6 shadow-sm"
        >
          <div className="space-y-6">
            <div className="grid grid-cols-2 gap-x-8 gap-y-5">
              <div className="grid grid-cols-[88px_minmax(0,1fr)] items-center gap-3">
                <Label htmlFor="widget-title" className="whitespace-nowrap">组件标题</Label>
                <Input
                  id="widget-title"
                  value={widgetTitle}
                  onChange={(e) => setWidgetTitle(e.target.value)}
                  maxLength={8}
                />
              </div>

              <div className="grid grid-cols-[88px_minmax(0,1fr)] items-center gap-3">
                <Label htmlFor="refresh" className="whitespace-nowrap">刷新间隔</Label>
                <Select value={refreshInterval} onValueChange={setRefreshInterval}>
                  <SelectTrigger id="refresh" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {REFRESH_OPTIONS.map((opt) => (
                      <SelectItem key={opt.value} value={String(opt.value)}>
                        {opt.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="col-span-2 grid grid-cols-[88px_minmax(0,1fr)] items-center gap-3">
                <Label className="whitespace-nowrap">涨跌颜色</Label>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={setUpColorRed}
                    className={cn(
                      'px-2 py-1 rounded-md border transition-all flex items-center gap-1 text-xs',
                      stockColor === 0 ? 'border-primary bg-primary/5' : 'border-border hover:border-muted-foreground/30'
                    )}
                  >
                    <ArrowUp className="h-3 w-3" style={{ color: '#ef4444' }} />
                    <span className="font-medium" style={{ color: '#ef4444' }}>红</span>
                    <span className="text-muted-foreground text-[10px]">/</span>
                    <ArrowDown className="h-3 w-3" style={{ color: '#22c55e' }} />
                    <span className="font-medium" style={{ color: '#22c55e' }}>绿</span>
                  </button>
                  <button
                    type="button"
                    onClick={setUpColorGreen}
                    className={cn(
                      'px-2 py-1 rounded-md border transition-all flex items-center gap-1 text-xs',
                      stockColor === 1 ? 'border-primary bg-primary/5' : 'border-border hover:border-muted-foreground/30'
                    )}
                  >
                    <ArrowUp className="h-3 w-3" style={{ color: '#22c55e' }} />
                    <span className="font-medium" style={{ color: '#22c55e' }}>绿</span>
                    <span className="text-muted-foreground text-[10px]">/</span>
                    <ArrowDown className="h-3 w-3" style={{ color: '#ef4444' }} />
                    <span className="font-medium" style={{ color: '#ef4444' }}>红</span>
                  </button>
                </div>
              </div>
            </div>

            <Separator />

            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <Label className="text-base flex items-center gap-2">
                  自选列表 ({localStocks.length})
                  {listLoading && (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                  )}
                </Label>
                <div className="flex items-center gap-2">
                  {localStocks.length > 0 && (
                    <Button
                      variant="outline"
                      onClick={() => refreshAllStocks(localStocks)}
                      disabled={listLoading}
                    >
                      {listLoading ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <RefreshCw className="h-4 w-4 mr-1.5" />
                      )}
                      {listLoading ? '刷新中' : '刷新行情'}
                    </Button>
                  )}
                  <StockSelect
                    onSelect={handleAddStock}
                    placeholder="搜索代码/名称添加"
                    existingCodes={localStocks.map((s) => s.code)}
                    clearOnSelect={false}
                  />
                </div>
              </div>
              <DataTable
                columns={stockColumns}
                data={localStocks}
                emptyContent={listLoading ? (
                  <span className="inline-flex items-center gap-2 text-sm opacity-50">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    加载中...
                  </span>
                ) : (
                  <span className="text-sm opacity-50">暂无自选股票</span>
                )}
              />
            </div>
          </div>
        </TabsContent>

        <TabsContent
          value="theme"
          className="rounded-xl border bg-card p-6 shadow-sm"
        >
          <WidgetThemeForm
            showSectionHeader={false}
            themeOption={stockThemeOption}
            value={widgetTheme}
            onChange={setWidgetTheme}
          />
        </TabsContent>
      </Tabs>
    </Window>
  )
}
