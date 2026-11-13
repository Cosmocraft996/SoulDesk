import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { stopOwnedChild } from './child-cleanup.mjs';
test('有界结束本次挂住的子进程，不影响另一个进程', async () => {
  const a = spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { windowsHide: true, stdio: 'ignore' });
  const b = spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { windowsHide: true, stdio: 'ignore' });
  try { await new Promise(resolve => setTimeout(resolve, 100)); const start = Date.now(); await stopOwnedChild(a, 100); assert.ok(Date.now() - start < 3000); assert.ok(a.exitCode !== null || a.signalCode !== null); assert.equal(b.exitCode, null); assert.equal(b.signalCode, null); }
  finally { await stopOwnedChild(a, 100); await stopOwnedChild(b, 100); }
});
