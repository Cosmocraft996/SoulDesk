export const hotspotVariants = [
  ['bangumi', 'bangumi/BangumiWidgetView.vue'],
  ['bilibili', 'bilibili/BilibiliWidgetView.vue'],
  ['bilibili-rcmd', 'bilibili-rcmd/BilibiliRcmdWidgetView.vue'],
  ['cls-telegraph', 'cls-telegraph/ClsTelegraphWidgetView.vue'],
  ['douyin', 'douyin/DouyinWidgetView.vue'],
  ['tou-tiao', 'tou-tiao/TouTiaoWidgetView.vue'],
  ['weibo', 'weibo/WeiBoWidgetView.vue'],
  ['world-cup', 'world-cup/WorldCupWidgetView.vue'],
  ['zhihu', 'zhihu/ZhihuWidgetView.vue'],
];
export const hotspotWidgets = hotspotVariants.map(([name]) => 'hotspot-' + name);
