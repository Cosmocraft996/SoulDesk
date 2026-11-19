// 固定登录状态检查只返回布尔值，不返回账号、Cookie或整个_PAGE。
(() => {
  try {
    return {
      origin: location.origin,
      loggedIn: location.origin === 'https://www.zhipin.com' && !!window._PAGE?.uid,
    };
  } catch {
    return { error: 'not_ready' };
  }
})()
