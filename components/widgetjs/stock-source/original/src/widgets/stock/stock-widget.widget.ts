import { Widget, WidgetKeyword } from '@widget-js/core'

const StockWidget = new Widget({
  name: '.stock',
  title: { 'zh-CN': '股票', 'en-US': 'Stock' },
  description: { 'zh-CN': '跟踪股票价格，每分钟刷新一次', 'en-US': 'Track stock prices, refresh every minute' },
  keywords: [WidgetKeyword.RECOMMEND],
  lang: 'zh-CN',
  width: 4,
  categories: ['utilities', 'finance'],
  height: 4,
  minWidth: 4,
  maxWidth: 5,
  minHeight: 3,
  maxHeight: 8,
  socialLinks: [
    { name: 'github', link: 'https://github.com/rtugeek/stock' },
  ],
  previewImage: '/preview_stock_large.png',
  path: '/widget/stock',
  configPagePath: '/widget/config/stock?transparent=false&frame=true&height=700&width=800',
  configPage: {
    path: '/widget/config/stock',
    transparent: false,
    frame: true,
    height: 900,
    resizable: true,
    width: 800,
  },
})

export default StockWidget
