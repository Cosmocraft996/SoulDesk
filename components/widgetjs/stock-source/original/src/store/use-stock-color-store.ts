import { create } from 'zustand'
import { persist, createJSONStorage } from 'zustand/middleware'
import Color from 'color'

type StockColorPreference = 0 | 1

interface StockColorState {
  stockColor: StockColorPreference
  toggleStockColor: () => void
  setUpColorRed: () => void
  setUpColorGreen: () => void
  isUpColorRed: () => boolean
  getUpColor: () => string
  getDownColor: () => string
  getColorByValue: (value: number | boolean) => { color: string, colorEnd: string }
}

const RED = '#f82842'
const GREEN = 'rgb(95,194,93)'

export const useStockColorStore = create<StockColorState>(
  // @ts-ignore
  persist(
    (set, get) => ({
      stockColor: 0,

      toggleStockColor: () => {
        set((state) => ({ stockColor: state.stockColor === 0 ? 1 : 0 }))
      },

      setUpColorRed: () => {
        set({ stockColor: 0 })
      },

      setUpColorGreen: () => {
        set({ stockColor: 1 })
      },

      isUpColorRed: () => {
        return get().stockColor === 0
      },

      getUpColor: () => {
        return get().stockColor === 0 ? RED : GREEN
      },

      getDownColor: () => {
        return get().stockColor === 0 ? GREEN : RED
      },

      getColorByValue: (value: number | boolean) => {
        const state = get()
        const isUp = typeof value === 'number' ? value >= 0 : value
        const color = isUp
          ? (state.stockColor === 0 ? RED : GREEN)
          : (state.stockColor === 0 ? GREEN : RED)
        return {
          color,
          colorEnd: Color(color).alpha(0).string(),
        }
      },
    }),
    {
      name: 'stock_color',
      storage: createJSONStorage(() => localStorage),
    },
  ),
)
