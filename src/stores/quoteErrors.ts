const XUEQIU_COOKIE_EXPIRED_HINT =
  "雪球 Cookie 可能已经过期，请到设置页面更新雪球 Cookie。";
const XUEQIU_API_FAILED_HINT =
  "访问雪球行情服务失败，请检查网络连接或稍后重试。";

export function toQuoteWarning(error: unknown): string {
  const detail = error instanceof Error ? error.message : String(error);
  const isXueqiuCookieExpired =
    detail.includes("Xueqiu API error") &&
    (detail.includes("400016") ||
      detail.includes("重新登录帐号后再试") ||
      detail.includes("刷新页面或者重新登录帐号后再试"));

  if (isXueqiuCookieExpired) return XUEQIU_COOKIE_EXPIRED_HINT;
  if (
    detail.includes("Xueqiu") ||
    detail.includes("xueqiu.com") ||
    detail.includes("stock.xueqiu.com")
  ) {
    return XUEQIU_API_FAILED_HINT;
  }
  return `行情获取失败：${detail}`;
}
