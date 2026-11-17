// 保留库原有的独立模块工作线程，避免将主界面脚本误当作worker执行。
export const WorkerUrl = new URL('./worker/subset-worker.chunk.js', location.href).href;
