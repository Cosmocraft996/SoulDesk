import type { Stock } from '@/model/stock'
import consola from 'consola'
import localforage from 'localforage'

const selfSelectStock = localforage.createInstance({
  name: 'selfSelectStock-v2',
})

class SelfSelectStockRepository {
  /**
   * 更新或者添加自选股
   * @param stock 股票信息
   */
  async save(stock: Stock): Promise<void> {
    try {
      const exists = await this.exists(stock.code)
      if (!exists) {
        // 新股票，设置排序顺序为当前最大值+1
        const stocks = await this.all()
        const maxOrder = Math.max(...stocks.map(s => s.sortOrder || 0), 0)
        stock.sortOrder = maxOrder + 1
      }
      await selfSelectStock.setItem(stock.code, stock)
    }
    catch (error) {
      consola.error('Failed to save stock:', error)
      throw new Error('保存自选股失败')
    }
  }

  /**
   * 更新股票列表顺序
   * @param stocks 排序后的股票列表
   */
  async saveOrder(stocks: Stock[]): Promise<void> {
    try {
      // 更新每个股票的sortOrder
      const updates = stocks.map((stock, index) => {
        stock.sortOrder = index
        return selfSelectStock.setItem(stock.code, stock)
      })
      await Promise.all(updates)
    }
    catch (error) {
      consola.error('Failed to update order:', error)
      throw new Error('更新股票顺序失败')
    }
  }

  /**
   * 列出所有自选股票
   * @returns 按sortOrder排列的自选股票列表
   */
  async all(): Promise<Stock[]> {
    try {
      const keys = await selfSelectStock.keys()
      const stocks = await Promise.all(
        keys.map(key => selfSelectStock.getItem<Stock>(key)),
      )
      return stocks
        .filter((stock): stock is Stock => stock !== null)
        .sort((a, b) => (a.sortOrder || 0) - (b.sortOrder || 0))
    }
    catch (error) {
      consola.error('Failed to get stocks:', error)
      return []
    }
  }

  async findByCode(code: string): Promise<Stock | undefined> {
    try {
      return await selfSelectStock.getItem<Stock>(code) || undefined
    }
    catch (error) {
      consola.error('Failed to get stock:', error)
      return undefined
    }
  }

  /**
   * 删除指定自选股
   * @param code 股票代码
   */
  async remove(code: string): Promise<void> {
    try {
      await selfSelectStock.removeItem(code)
    }
    catch (error) {
      consola.error('Failed to remove stock:', error)
      throw new Error('删除自选股失败')
    }
  }

  /**
   * 检查股票是否已在自选列表中
   * @param code 股票代码
   */
  async exists(code: string): Promise<boolean> {
    const stock = await this.findByCode(code)
    return !!stock
  }

  /**
   * 清空所有自选股
   */
  async clear(): Promise<void> {
    try {
      await selfSelectStock.clear()
    }
    catch (error) {
      consola.error('Failed to clear stocks:', error)
      throw new Error('清空自选股失败')
    }
  }
}

export const selfSelectStockRepository = new SelfSelectStockRepository()
