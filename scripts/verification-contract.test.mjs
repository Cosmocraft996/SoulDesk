import test from 'node:test';
import assert from 'node:assert/strict';
import { validateReportContract } from './verification-contract.mjs';
test('G3入口拒绝缺失资源阶段或快捷键冲突检查的完成报告', () => {
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'g3', version: '0.3.2', programChecksComplete: true, g3ChecksComplete: true, checks: [] }, 'g3', '0.3.2'));
});
test('G2入口拒绝缺少业务检查或沿用G1的报告', () => {
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'g2', version: '0.3.0', programChecksComplete: true, checks: [] }, 'g2', '0.3.0'));
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'g1', version: '0.3.0', programChecksComplete: true, g1ChecksComplete: true }, 'g2', '0.3.0'));
});
test('G1入口拒绝旧G0程序与未完成G1检查的报告', () => {
  assert.throws(() => validateReportContract({ programChecksComplete: true, stage: '程序检查完成' }, 'g1', '0.2.0'));
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'g0', version: '0.2.0', programChecksComplete: true }, 'g1', '0.2.0'));
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'g1', version: '0.2.0', programChecksComplete: true, checks: [] }, 'g1', '0.2.0'));
});
test('报告必须属于当前工程版本', () => {
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'development-runtime', version: '0.2.0', g1ChecksComplete: true }, 'g1', '0.2.0'));
  assert.throws(() => validateReportContract({ verificationProtocol: 2, phase: 'g0', version: '0.1.0' }, 'g0', '0.2.0'));
  assert.doesNotThrow(() => validateReportContract({ verificationProtocol: 2, phase: 'g0', version: '0.2.0' }, 'g0', '0.2.0'));
});
