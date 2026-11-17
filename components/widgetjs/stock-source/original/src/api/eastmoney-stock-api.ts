import type { OpenDataResult } from '@/api/opendata'
import type { Stock } from '@/model/stock'
import { Quotation } from '@/model/quotation'
import axios, { type AxiosRequestConfig } from 'axios'

const CACHE_TTL_MS = 60_000
const cache = new Map<string, { expireAt: number; value: any }>()

function cacheKey(url: string, params?: Record<string, any>): string {
  if (!params || Object.keys(params).length === 0) return url
  const sorted = Object.keys(params)
    .sort()
    .map((k) => `${k}=${JSON.stringify(params[k])}`)
  return `${url}?${sorted.join('&')}`
}

async function cachedGet<T = any>(url: string, config?: AxiosRequestConfig): Promise<T> {
  const key = cacheKey(url, config?.params as Record<string, any> | undefined)
  const hit = cache.get(key)
  const now = Date.now()
  if (hit && hit.expireAt > now) {
    return hit.value as T
  }
  const response = await axios.get(url, config)
  cache.set(key, { expireAt: now + CACHE_TTL_MS, value: response.data })
  return response.data as T
}

function baiduExchangeToEmSecid(code: string, exchange?: string): string {
  if (exchange) {
    const ex = exchange.toUpperCase()
    if (ex === 'SH') return `1.${code}`
    if (ex === 'SZ') return `0.${code}`
    if (ex === 'BJ') return `0.${code}`
    if (ex === 'HK') return `116.${code}`
    if (ex === 'US' || ex === 'NASDAQ' || ex === 'NSD' || ex === 'NYSE') return `105.${code}`
  }
  if (/^\d{6}$/.test(code)) {
    return code.startsWith('6') || code.startsWith('9') ? `1.${code}` : `0.${code}`
  }
  if (/^\d{5}$/.test(code)) {
    return `116.${code}`
  }
  return `105.${code}`
}

function emMarketToBaiduExchange(mk: number, type?: string, code?: string): string {
  if (type === 'index') {
    if (mk === 1) return 'SH'
    if (mk === 0) return 'SZ'
    if (mk === 116) return 'HK'
    return 'US'
  }
  if (mk === 1) return 'SH'
  if (mk === 0) return 'SZ'
  if (mk === 116) return 'HK'
  if (mk === 105 || mk === 106 || mk === 107) {
    const u = code?.toUpperCase?.() || ''
    if (u === '.DJI' || u === '.IXIC' || u === '.INX') return 'US'
    return 'NASDAQ'
  }
  return 'SH'
}

function emMarketToBaiduMarket(mk: number): string {
  if (mk === 1 || mk === 0) return 'cn'
  if (mk === 116) return 'hk'
  return 'us'
}

function emStatusToBaiduInfo(tr: string): 'TRADE' | 'ENDTR' | 'STOPT' {
  if (tr === '1') return 'TRADE'
  if (tr === '2') return 'ENDTR'
  return 'STOPT'
}

function sign(x: number): '1' | '-1' | '0' {
  if (x > 0) return '1'
  if (x < 0) return '-1'
  return '0'
}

interface EmQuoteKlt1Item {
  f57: string
  f58: string
  f43: number
  f44: number
  f45: number
  f46: number
  f47: number
  f48: number
  f50: string
  f51: string
  f52: number
  f60: number
  f116: number
  f117: number
  f162: number
  f167: number
  f168: number
  f169: number
  f170: number
  f171: number
  f177: number
  f178: string
}

function emQuoteToStock(q: EmQuoteKlt1Item, mk: number): Stock {
  const price = q.f43 / Math.pow(10, q.f52 - 0)
  const preClose = q.f60 / Math.pow(10, q.f52 - 0)
  const increase = price - preClose
  const ratio = preClose === 0 ? 0 : ((increase / preClose) * 100)
  return {
    code: q.f57,
    type: 'stock',
    market: emMarketToBaiduMarket(mk),
    follow_status: '0',
    amount: String(q.f48 / 10000),
    exchange: emMarketToBaiduExchange(mk, 'stock', q.f57),
    name: q.f58,
    price: price.toFixed(q.f52),
    increase: increase.toFixed(q.f52),
    ratio: `${ratio.toFixed(2)}%`,
    amplitudeRatio: q.f170 ? String(q.f170 / 100) : '0',
    turnoverRatio: q.f168 ? String(q.f168 / 100) : '0',
    holdingAmount: '0',
    volume: String(q.f47 / 100),
    capitalization: q.f116 ? String(q.f116 / 100000000) : '0',
    peRate: q.f167 ? String(q.f167 / 100) : '0',
    pbRate: q.f167 ? String(q.f167 / 100) : (q.f117 && q.f116 ? (q.f117 / q.f116 * 100 / 100).toFixed(2) : '0'),
    status: sign(increase),
    stockStatus: q.f178 === '1' ? '2' : '6',
    stockStatusInfo: emStatusToBaiduInfo(q.f178),
    src_loc: String(mk),
    subType: '',
    sf_url: '',
    pv: '0',
    CNYPrice: price.toFixed(q.f52),
  }
}

export class EastMoneyStockApi {
  static async getStock(code: string): Promise<Stock | undefined> {
    return this.getByType(code, 'stock')
  }

  static async getFund(code: string): Promise<Stock | undefined> {
    return this.getByType(code, 'fund')
  }

  static async getByType<T>(code: string, type?: StockType): Promise<T | undefined> {
    const secid = baiduExchangeToEmSecid(code)
    try {
      const data = await cachedGet<string>(
        `https://push2.eastmoney.com/api/qt/stock/get?secid=${secid}&fields=f43,f44,f45,f46,f47,f48,f50,f51,f52,f57,f58,f60,f116,f117,f162,f167,f168,f169,f170,f171,f177,f178&ut=fa5fd1943c7b386f172d6893dbfba10b&invt=2&fltt=2`
      )
      const parsed = data as unknown as { data: EmQuoteKlt1Item | null }
      if (!parsed?.data) return undefined
      const mk = Number.parseInt(secid.split('.')[0], 10)
      const stock = emQuoteToStock(parsed.data, mk)
      if (type && stock.type !== type) return undefined
      stock.code = code
      return stock as unknown as T
    } catch {
      return undefined
    }
  }

  static async getIndex(code: string): Promise<Stock | undefined> {
    const result = await this.selfSelect(code, true)
    if (result && result.length > 0) {
      return result.find((s: Stock) => s.type === 'index') ?? result[0]
    }
    return undefined
  }

  static async getRawMinuteTrends(code: string): Promise<EmMinuteRaw | undefined> {
    const secid = baiduExchangeToEmSecid(code)
    try {
      const raw = await cachedGet<string>(
        `https://push2his.eastmoney.com/api/qt/stock/trends2/get?fields1=f1,f2,f3,f4,f5,f6,f7,f8,f9,f10,f11,f12,f13,f14,f15,f16,f17,f18,f19,f20,f21,f22,f23,f24,f25,f26,f27,f28,f29,f30,f31,f32,f33,f34,f35,f36,f37,f38,f39,f40,f41,f42,f43,f44,f45,f46,f47,f48,f49,f50&fields2=f51,f52,f53,f54,f55,f56,f57,f58&secid=${secid}&klt=1&ndays=1&iscr=0&ut=fa5fd1943c7b386f172d6893dbfba10b`
      )
      const json = raw as any
      if (!json?.data) return undefined
      return json.data as EmMinuteRaw
    } catch {
      return undefined
    }
  }

  static async selfSelect(query: string, queryDetail = false): Promise<Stock[]> {
    if (!query || query.length < 1) return []
    try {
      const raw = await cachedGet<EmSearchSuggestResponse>(
        `https://searchapi.eastmoney.com/api/suggest/get?input=${encodeURIComponent(query)}&type=14&token=D43BF722C8E33BDC906FB84D85E326E8&count=20`
      )
      const table = raw.QuotationCodeTable
      if (!table || table.Status !== 0) return []
      const arr: EmSearchSuggestDataItem[] = table.Data ?? []
      if (arr.length === 0) return []

      const baseArr: Stock[] = arr.slice(0, 20).map((item) => {
        const mkStr = item.MktNum ?? (item.QuoteID ? String(item.QuoteID).split('.')[0] : '1')
        const mk = Number.parseInt(String(mkStr), 10)
        const st = item.SecurityTypeName ?? ''
        const classify = item.Classify ?? ''
        let type: 'index' | 'fund' | 'stock' | 'block' = 'stock'
        if (st.includes('指数') || classify === 'Index') type = 'index'
        else if (st.includes('基金') || classify === 'Fund') type = 'fund'
        else if (st.includes('板块') || classify === 'Block') type = 'block'
        return {
          code: item.Code,
          type,
          market: emMarketToBaiduMarket(mk),
          follow_status: '0',
          amount: '0',
          exchange: emMarketToBaiduExchange(mk, type === 'index' ? 'index' : 'stock', item.Code),
          name: item.SecurityTypeName,
          price: '--',
          increase: '0',
          ratio: '0',
          amplitudeRatio: '0',
          turnoverRatio: '0',
          holdingAmount: '0',
          volume: '0',
          capitalization: '0',
          peRate: '0',
          pbRate: '0',
          status: '0',
          stockStatus: '6',
          stockStatusInfo: 'STOPT',
          src_loc: String(mk),
          subType: '',
          sf_url: '',
          pv: '0',
          CNYPrice: '--',
        }
      })

      if (!queryDetail) {
        return baseArr
      }

      const secids = arr.slice(0, 20).map((item) => item.QuoteID ?? `${item.MktNum}.${item.Code}`).join(',')
      const details = await cachedGet<{ data?: { diff?: EmQuoteKlt1Item[] } }>(
        `https://push2.eastmoney.com/api/qt/ulist.np/get?fltt=2&fields=f43,f44,f45,f46,f47,f48,f50,f51,f52,f57,f58,f60,f116,f117,f162,f167,f168,f169,f170,f171,f177,f178&secids=${secids}&ut=fa5fd1943c7b386f172d6893dbfba10b`
      )
      const rows: EmQuoteKlt1Item[] = details?.data?.diff ?? []
      if (rows.length === 0) return baseArr
      const mergedArr: Stock[] = []
      for (const r of rows) {
        const meta = arr.find((m) => m.Code === r.f57)
        const base = baseArr.find((b) => b.code === r.f57)
        const mkStr = meta?.MktNum ?? (meta?.QuoteID ? String(meta.QuoteID).split('.')[0] : '1')
        const mk = Number.parseInt(String(mkStr), 10)
        const s = emQuoteToStock(r, mk)
        if (meta?.SecurityTypeName) {
          const st = meta.SecurityTypeName
          const classify = meta.Classify ?? ''
          if (st.includes('指数') || classify === 'Index') s.type = 'index'
          else if (st.includes('基金') || classify === 'Fund') s.type = 'fund'
          else if (st.includes('板块') || classify === 'Block') s.type = 'block'
          else s.type = 'stock'
        }
        s.code = r.f57
        s.sortOrder = base?.sortOrder
        s.holdingPrice = base?.holdingPrice
        s.holdingShares = base?.holdingShares
        mergedArr.push(s)
      }
      if (mergedArr.length > 0) return mergedArr
      return baseArr
    } catch {
      return []
    }
  }

  static async getQuotation(code: string): Promise<BaiDuApiResponse<QuotationResult>> {
    const secid = baiduExchangeToEmSecid(code)
    const quote = await this.getByType<Stock>(code)
    const mk = Number.parseInt(secid.split('.')[0], 10)
    const marketData: MarketData[] = []
    try {
      const minuteData = await cachedGet<string>(
        `https://push2his.eastmoney.com/api/qt/stock/trends2/get?fields1=f1,f2,f3,f4,f5,f6,f7,f8,f9,f10,f11,f12,f13&fields2=f51,f52,f53,f54,f55,f56,f57,f58&secid=${secid}&klt=1&ndays=1&iscr=0&ut=fa5fd1943c7b386f172d6893dbfba10b`
      )
      const m = minuteData as any
      const trends: string[] = m?.data?.trends ?? []
      for (const t of trends) {
        const parts = t.split(',')
        const date = parts[0] ?? ''
        const price = parts[2] ?? '0'
        const volume = parts[5] ?? '0'
        const avgPrice = parts[7] ?? '0'
        const p = `${date},${price},${avgPrice},${volume}`
        marketData.push({ date, p })
      }
    } catch {
      /* noop */
    }
    const timeText = quote?.stockStatusInfo === 'TRADE' ? '交易中' : '休市'
    return {
      QueryID: code,
      ResultCode: '0',
      Result: {
        StdStg: '',
        StdStl: '',
        _update_time: timeText,
        code,
        url: '',
        wiseUrl: '',
        encode: '',
        key: code,
        provider: 'eastmoney',
        update: {
          timezone: 'Asia/Shanghai',
          text: timeText,
          time: Math.floor(Date.now() / 1000),
          time_diff: '8',
        },
        newMarketData: {
          headers: ['时间', '价格', '均价', '成交量'],
          keys: ['date', 'p'],
          marketData,
        },
      },
    }
  }

  static async getQuotationMinute(code: string, group: QuotationGroup = 'quotation_minute_ab'): Promise<Quotation> {
    void group
    const secid = baiduExchangeToEmSecid(code)
    const mk = Number.parseInt(secid.split('.')[0], 10)

    let quoteInfo: EmQuoteKlt1Item | null = null
    try {
      const qData = await cachedGet<string>(
        `https://push2.eastmoney.com/api/qt/stock/get?secid=${secid}&fields=f43,f44,f45,f46,f47,f48,f50,f51,f52,f57,f58,f60,f116,f117,f162,f167,f168,f169,f170,f171,f177,f178&ut=fa5fd1943c7b386f172d6893dbfba10b&invt=2&fltt=2`
      )
      const json = qData as unknown as { data: EmQuoteKlt1Item | null }
      quoteInfo = json?.data ?? null
    } catch {
      quoteInfo = null
    }

    const marketData: MarketData[] = []
    let preClose = 0
    try {
      const minuteData = await cachedGet<string>(
        `https://push2his.eastmoney.com/api/qt/stock/trends2/get?fields1=f1,f2,f3,f4,f5,f6,f7,f8,f9,f10,f11,f12,f13&fields2=f51,f52,f53,f54,f55,f56,f57,f58&secid=${secid}&klt=1&ndays=1&iscr=0&ut=fa5fd1943c7b386f172d6893dbfba10b`
      )
      const m = minuteData as any
      preClose = Number(m?.data?.preKClose ?? 0) / 100 || Number(m?.data?.jqPreClose ?? 0) / 100 || 0
      const trends: string[] = m?.data?.trends ?? []
      for (const t of trends) {
        const parts = t.split(',')
        const date = parts[0] ?? ''
        const price = parts[2] ?? '0'
        const avgPrice = parts[7] ?? '0'
        const volume = parts[5] ?? '0'
        const amount = parts[6] ?? '0'
        const p = `${date},${price},${avgPrice},${volume},${amount}`
        marketData.push({ date, p })
      }
    } catch {
      /* noop */
    }

    const dp = quoteInfo?.f52 ?? 2
    const priceRaw = quoteInfo?.f43 ?? 0
    const price = priceRaw / Math.pow(10, dp)
    const vol = quoteInfo?.f47 ?? 0
    const amount = quoteInfo?.f48 ?? 0
    const close = quoteInfo?.f60 ?? preClose * Math.pow(10, dp)
    const preClosePrice = close / Math.pow(10, dp) || preClose
    const increase = price - preClosePrice
    const ratioPct = preClosePrice === 0 ? 0 : (increase / preClosePrice) * 100
    const info = emStatusToBaiduInfo(quoteInfo?.f178 ?? '')
    const exchange = emMarketToBaiduExchange(mk, 'stock', quoteInfo?.f57)

    return new Quotation({
      pankouinfos: {
        indicatorTitle: '盘口',
        indicatorUrl: '',
        list: [
          { ename: 'preClose', name: '昨收', value: preClosePrice.toFixed(dp), originValue: String(close) },
          { ename: 'open', name: '今开', value: quoteInfo ? (quoteInfo.f46 / Math.pow(10, dp)).toFixed(dp) : '--', originValue: String(quoteInfo?.f46 ?? 0) },
          { ename: 'high', name: '最高', value: quoteInfo ? (quoteInfo.f44 / Math.pow(10, dp)).toFixed(dp) : '--', originValue: String(quoteInfo?.f44 ?? 0) },
          { ename: 'low', name: '最低', value: quoteInfo ? (quoteInfo.f45 / Math.pow(10, dp)).toFixed(dp) : '--', originValue: String(quoteInfo?.f45 ?? 0) },
          { ename: 'volume', name: '成交量', value: `${(vol / 10000).toFixed(2)}万手`, originValue: String(vol) },
          { ename: 'amount', name: '成交额', value: amount >= 100000000 ? `${(amount / 100000000).toFixed(2)}亿` : `${(amount / 10000).toFixed(2)}万`, originValue: String(amount) },
          { ename: 'turnoverRate', name: '换手', value: quoteInfo?.f168 ? `${(quoteInfo.f168 / 100).toFixed(2)}%` : '--', originValue: String(quoteInfo?.f168 ?? 0) },
          { ename: 'amplitude', name: '振幅', value: quoteInfo?.f170 ? `${(quoteInfo.f170 / 100).toFixed(2)}%` : '--', originValue: String(quoteInfo?.f170 ?? 0) },
        ],
      },
      askinfos: [],
      buyinfos: [],
      update: {
        text: info === 'TRADE' ? '交易中' : '休市',
        time: String(Math.floor(Date.now() / 1000)),
        realUpdateTime: new Date().toLocaleString('zh-CN', { hour12: false }),
        timezone: 'Asia/Shanghai',
        shortZone: 'CST',
        time_diff: 8,
        stockStatus: quoteInfo?.f178 ?? '',
        tradeStatus: info,
        tradeStatusCN: info === 'TRADE' ? '交易中' : (info === 'ENDTR' ? '已收盘' : '休市'),
      },
      cur: {
        time: Math.floor(Date.now() / 1000),
        price: price.toFixed(dp),
        ratio: `${ratioPct.toFixed(2)}%`,
        increase: increase.toFixed(dp),
        volume: String(Math.floor(vol / 100)),
        avgPrice: '--',
        timeKey: '',
        amount: amount >= 100000000 ? `${(amount / 100000000).toFixed(2)}亿` : `${(amount / 10000).toFixed(2)}万`,
        show: 1,
        unit: '',
      },
      basicinfos: {
        exchange,
        code,
        name: quoteInfo?.f58 ?? code,
        stockStatus: quoteInfo?.f178 ?? '',
        stock_market_code: String(mk),
        stockCode: code,
        tradeStatus: info,
        tradeStatusCN: info === 'TRADE' ? '交易中' : (info === 'ENDTR' ? '已收盘' : '休市'),
      },
      tag_list: [],
      chartTabs: [],
      tradingCounters: [],
      foreign_key: code,
      releaseDate: '',
      useWS: '0',
      financeType: 'stock',
      newMarketData: {
        headers: ['时间', '价格', '均价', '成交量', '成交额'],
        keys: ['date', 'p'],
        marketData,
      },
    })
  }

  static async getOpenData(query: string, month: number): Promise<OpenDataResult | undefined> {
    void query
    void month
    return undefined
  }
}

export type QuotationGroup = 'quotation_minute_ab' | 'quotation_index_fiveday' | 'quotation_block_minute'
export type StockType = 'index' | 'fund' | 'stock' | 'block'

export interface EmMinuteRaw {
  code: string
  market: number
  type: number
  status: number
  name: string
  decimal: number
  preSettlement: number
  preClose: number
  beticks: string
  trendsTotal: number
  time: number
  kind: number
  prePrice: number
  trends: string[]
}

export interface LabelMap {
  text: string
  ename: string
}

export interface EmSearchSuggestDataItem {
  Code: string
  Name: string
  PinYin: string
  ID: string
  JYS: string
  Classify: string
  MarketType: string
  SecurityTypeName: string
  SecurityType: string
  MktNum: string
  TypeUS: string
  QuoteID: string
  UnifiedCode: string
  InnerCode: string
}

export interface EmSearchSuggestResponse {
  QuotationCodeTable: {
    Data: EmSearchSuggestDataItem[]
    Status: number
    Message: string
    TotalCount: number
    BizCode: string
    BizMsg: string
  }
}

export interface StockStatus {
  is_trend: string
  time_sort: string
}

export interface Result {
  stock: Stock[]
  index: any[]
  deal_status: string
  stock_status: StockStatus
  refresh_time: string
  labelMap: LabelMap[]
  isNew: string
  follow_num: string
}

export interface BaiDuApiResponse<T> {
  QueryID: string
  ResultCode: string | number
  Result: T
}

interface MarketData {
  date: string
  p: string
}

export interface QuotationResult {
  StdStg: string
  StdStl: string
  _update_time: string
  code: string
  url: string
  wiseUrl: string
  encode: string
  key: string
  provider: string
  update: {
    timezone: string
    text: string
    time: number
    time_diff: string
  }
  newMarketData: NewMarketData
}

export interface NewMarketData {
  headers: string[]
  keys: string[]
  marketData: MarketData[]
}
