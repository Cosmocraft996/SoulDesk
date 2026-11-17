import type { StockType } from '@/api/eastmoney-stock-api'

export interface Stock {
  code: string
  type: StockType
  market: string
  follow_status: string
  amount: string
  exchange: string
  name: string
  price: string
  increase: string
  ratio: string
  amplitudeRatio: string
  turnoverRatio: string
  holdingAmount: string
  volume: string
  capitalization: string
  peRate: string
  pbRate: string
  /**
   * 0: 休市/停牌
   * 1: 涨
   * -1: 跌
   */
  status: string
  /**
   * 2 - 交易中
   * 6 - 停牌/休市
   */
  stockStatus: string
  stockStatusInfo: 'STOPT' | 'ENDTR' | 'TRADE'
  src_loc: string
  subType: string
  sf_url: string
  pv: string
  CNYPrice: string
  /**
   *  排序顺序，这个是自己的字段
   */
  sortOrder?: number
  /**
   * 持仓价
   */
  holdingPrice?: number
  /**
   * 持仓数量（股）
   */
  holdingShares?: number
}
