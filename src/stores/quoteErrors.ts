const XUEQIU_COOKIE_EXPIRED_HINT =
  "雪球 Cookie 已过期，请到 设置 → 通用设置 → 雪球 Cookie 设置 中设置 Cookie。";
const XUEQIU_API_FAILED_HINT =
  "访问雪球行情服务失败，请检查网络连接或稍后重试。";

export function toQuoteWarning(error: unknown): string {
  const detail = error instanceof Error ? error.message : String(error);
  // These errors are emitted only after receiving an HTTP or API error response.
  // Transport failures have separate prefixes and keep the service warning.
  const isXueqiuResponseError =
    detail.includes("Xueqiu API error") ||
    detail.includes("Failed to initialize Xueqiu token: HTTP ");

  if (isXueqiuResponseError) return XUEQIU_COOKIE_EXPIRED_HINT;
  if (
    detail.includes("Xueqiu") ||
    detail.includes("xueqiu.com") ||
    detail.includes("stock.xueqiu.com")
  ) {
    return XUEQIU_API_FAILED_HINT;
  }
  return `行情获取失败：${detail}`;
}
