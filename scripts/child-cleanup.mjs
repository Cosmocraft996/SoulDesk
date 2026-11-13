// 只处理调用方刚启动并持有句柄的子进程，不按名字批量结束进程。
const exited = child => child.exitCode !== null || child.signalCode !== null;
async function waitExit(child, timeout) {
  if (exited(child)) return true;
  return new Promise(resolve => {
    const done = () => finish(true);
    const timer = setTimeout(() => finish(exited(child)), timeout);
    function finish(value) { clearTimeout(timer); child.off('exit', done); resolve(value); }
    child.once('exit', done);
    if (exited(child)) finish(true);
  });
}
export async function stopOwnedChild(child, graceMs = 2000) {
  if (!child.pid || exited(child)) return;
  if (await waitExit(child, graceMs)) return;
  child.kill();
  if (await waitExit(child, 1000)) return;
  child.kill('SIGKILL');
  if (!(await waitExit(child, 1000))) { child.unref(); throw new Error(`本次测试子进程 ${child.pid} 未按时退出，请保留报告后人工检查`); }
}
