const g1Checks = [
  'G1兼容升级保留实例配置并补默认项',
  'G1脚本错误坏包未知能力及配置不兼容均保留旧版',
  'G1同一主程序导入第二种包双实例及布局保存',
  'G1实际窗口销毁重建恢复逻辑尺寸浮窗归属和置顶',
  'G1反复切换12次会话数量有界',
];
const g2Checks = ['G2后台启动保持隐藏', 'G2外部包真实行情双实例与表单保存', 'G2工作台浮窗复用与移除单实例订阅隔离', 'G2零视图真实行情警报与独立递送', 'G2十次受控重连不重播单次事件', 'G2当前数据库备份完整性与版本检查'];
export function validateReportContract(report, phase, version) {
  if (report.verificationProtocol !== 2 || report.phase !== phase || report.version !== version) throw new Error('已安装程序的验收协议、阶段或版本不匹配，请安装当前构建的安装包。');
  if (report.programChecksComplete && phase === 'g1' && (report.g1ChecksComplete !== true || g1Checks.some(name => !report.checks?.some(check => check.name === name)))) throw new Error('报告缺少完整 G1 检查，不能作为 G1 完成证据。');
  if (report.programChecksComplete && phase === 'g2' && (report.g2ChecksComplete !== true || g2Checks.some(name => !report.checks?.some(check => check.name === name)))) throw new Error('报告缺少完整 G2 业务检查，不能作为 G2 完成证据。');
  if (report.programChecksComplete && phase === 'g3' && (report.g3ChecksComplete !== true || ['G3快捷键冲突保留旧键', 'G3本机屏幕外窗口找回', 'G3视图进程树资源与回收'].some(name => !report.checks?.some(check => check.name === name)) || ['visible', 'hidden', 'destroyed', 'reopened'].some(phase => !report.resourcePhases?.some(p => p.phase === phase && p.samples >= 7 && p.seconds >= 30)))) throw new Error('报告缺少G3快捷键、找回窗口或完整视图资源阶段，不能作为G3完成证据。');
}
