import { create } from 'zustand'
import type { Stock } from '@/model/stock'
import type { StockType } from '@/api/bai-du-stock-api'
import { selfSelectStockRepository } from '@/data/self-select-stock-repository'

interface BroadcastEvent {
  event: 'save' | 'saveOrder' | 'delete'
  payload: any
}

interface SelfSelectStockState {
  stocks: Stock[]
  loading: boolean
  loadAll: () => Promise<void>
  saveStock: (stock: Stock) => Promise<void>
  saveOrder: (stocks: Stock[]) => Promise<void>
  deleteStock: (code: string) => Promise<void>
  clearAll: () => Promise<void>
  selectStocksByTypes: (types: StockType[]) => Stock[]
}

let broadcastChannel: BroadcastChannel | null = null

function getBroadcastChannel(): BroadcastChannel | null {
  if (typeof window === 'undefined') {
    return null
  }
  if (!broadcastChannel) {
    try {
      broadcastChannel = new BroadcastChannel('self-select-stock')
    }
    catch {
      return null
    }
  }
  return broadcastChannel
}

export const useSelfSelectStockStore = create<SelfSelectStockState>((set, get) => ({
  stocks: [],
  loading: false,

  loadAll: async () => {
    set({ loading: true })
    try {
      const stocks = await selfSelectStockRepository.all()
      set({ stocks })
    }
    finally {
      set({ loading: false })
    }
  },

  saveStock: async (stock: Stock) => {
    await selfSelectStockRepository.save(stock)
    const stocks = await selfSelectStockRepository.all()
    set({ stocks })

    const channel = getBroadcastChannel()
    if (channel) {
      const event: BroadcastEvent = {
        event: 'save',
        payload: JSON.parse(JSON.stringify(stock)),
      }
      channel.postMessage(event)
    }
  },

  saveOrder: async (stocks: Stock[]) => {
    const rawData = JSON.parse(JSON.stringify(stocks))
    await selfSelectStockRepository.saveOrder(rawData)
    const updatedStocks = await selfSelectStockRepository.all()
    set({ stocks: updatedStocks })

    const channel = getBroadcastChannel()
    if (channel) {
      const event: BroadcastEvent = {
        event: 'saveOrder',
        payload: rawData,
      }
      channel.postMessage(event)
    }
  },

  deleteStock: async (code: string) => {
    await selfSelectStockRepository.remove(code)
    const stocks = await selfSelectStockRepository.all()
    set({ stocks })

    const channel = getBroadcastChannel()
    if (channel) {
      const event: BroadcastEvent = {
        event: 'delete',
        payload: { code },
      }
      channel.postMessage(event)
    }
  },

  clearAll: async () => {
    await selfSelectStockRepository.clear()
    set({ stocks: [] })
  },

  selectStocksByTypes: (types: StockType[]) => {
    return get().stocks.filter(stock => types.includes(stock.type))
  },
}))

if (typeof window !== 'undefined') {
  const channel = getBroadcastChannel()
  if (channel) {
    channel.onmessage = async () => {
      await useSelfSelectStockStore.getState().loadAll()
    }
  }
}
