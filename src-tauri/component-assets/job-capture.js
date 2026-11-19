// 由宿主内部按固定动作执行；没有页面传入的代码、选择器、账号或脚本参数。
(() => {
  try {
    if (location.origin !== 'https://www.zhipin.com' || location.pathname !== '/web/geek/job') {
      return { error: 'wrong_page' };
    }
    const loggedIn = !!window._PAGE?.uid;
    if (!loggedIn) return { error: 'login_required' };
    const page = document.querySelector('.page-job-wrapper')?.__vue__;
    if (!page || !Array.isArray(page.jobList) || !page.pageVo) return { error: 'not_ready' };
    if (page.jobList.length > 100) return { error: 'capacity_exceeded' };
    const pageNumber = page.pageVo.page, pageSize = page.pageVo.pageSize, total = page.jobTotal;
    if (!Number.isSafeInteger(pageNumber) || pageNumber < 1 || pageNumber > 10000
      || !Number.isSafeInteger(pageSize) || pageSize < 1 || pageSize > 100
      || !Number.isSafeInteger(total) || total < 0) return { error: 'invalid_metadata' };
    const expected = Math.min(pageSize, Math.max(0, total - (pageNumber - 1) * pageSize));
    if (page.jobList.length !== expected) return { error: 'not_ready' };
    const fields = [
      'encryptJobId', 'areaDistrict', 'brandName', 'brandLogo', 'jobName', 'jobLabels',
      'skills', 'jobExperience', 'salaryDesc', 'jobDegree', 'cityName', 'businessDistrict',
      'brandStageName', 'brandIndustry', 'brandScaleName', 'welfareList',
      'daysPerWeekDesc', 'leastMonthDesc',
    ];
    const lists = new Set(['jobLabels', 'skills', 'welfareList']);
    const validText = (value, max) => typeof value === 'string' && value.length <= max * 2
      && Array.from(value).length <= max && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(value);
    const jobs = [];
    for (const job of page.jobList) {
      if (!job || typeof job !== 'object' || Array.isArray(job)) return { error: 'invalid_data' };
      const result = {};
      for (const key of fields) {
        const value = job[key];
        if (value == null) result[key] = null;
        else if (lists.has(key)) {
          if (!Array.isArray(value) || value.length > 100 || !value.every(item => validText(item, 300))) return { error: 'invalid_data' };
          result[key] = Array.from(value);
        } else {
          const max = key === 'encryptJobId' ? 160 : key === 'jobName' ? 500 : key === 'brandLogo' ? 2048 : 300;
          if (!validText(value, max)) return { error: 'invalid_data' };
          result[key] = value;
        }
      }
      jobs.push(result);
    }
    const result = {
      origin: location.origin, path: location.pathname, loggedIn,
      page: pageNumber, pageSize, total,
      jobs,
    };
    // 限制实际跨进程返回值；不截断岗位，超限整页报错。
    return JSON.stringify(result).length > 512 * 1024 ? { error: 'capacity_exceeded' } : result;
  } catch {
    return { error: 'capture_failed' };
  }
})()
