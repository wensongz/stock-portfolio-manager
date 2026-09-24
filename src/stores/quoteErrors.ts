const XUEQIU_COOKIE_EXPIRED_HINT =
  "雪球 Cookie 已过期，请到 设置 → 通用设置 → 雪球 Cookie 设置 中设置 Cookie。";
const XUEQIU_API_FAILED_HINT =
  "访问雪球行情服务失败，请检查网络连接或稍后重试。";

function xueqiuBusinessError(detail: string): { code: number; description: string } | null {
  if (!detail.startsWith("Xueqiu API error")) return null;

  const fields = detail.match(/^Xueqiu API error for .+?: code=(-?\d+), message=([\s\S]*)$/)
    ?? detail.match(/^Xueqiu API error (-?\d+):\s*([\s\S]*)$/);
  if (fields) {
    const code = Number(fields[1]);
    return code === 0 ? null : { code, description: fields[2].trim() };
  }

  const response = detail.indexOf(". Response: ");
  if (response === -1) return null;
  try {
    const body: unknown = JSON.parse(detail.slice(response + ". Response: ".length));
    if (!body || typeof body !== "object" || !("error_code" in body)) return null;
    const code = body.error_code;
    if (typeof code !== "number" && typeof code !== "string") return null;
    if (!/^-?\d+$/.test(String(code)) || Number(code) === 0) return null;
    const description = "error_description" in body
      ? body.error_description
      : "description" in body ? body.description : "";
    return { code: Number(code), description: typeof description === "string" ? description.trim() : "" };
  } catch {
    // HTTP bodies may be HTML, empty, or truncated; the status remains useful.
    return null;
  }
}

export function toQuoteWarning(error: unknown): string {
  const originalDetail = error instanceof Error ? error.message : String(error);
  // A secondary provider's failure must not change the primary error category.
  const detail = originalDetail.split("; fallback failed:", 1)[0];
  if (!detail.includes("Xueqiu") && !detail.includes("xueqiu.com")) {
    return `行情获取失败：${originalDetail}`;
  }

  const invalidCnSymbol = detail.match(/^Unknown CN market prefix '[^']*' in symbol (.*) for Xueqiu$/)
    ?? detail.match(/^Invalid CN symbol for Xueqiu: (.*)$/);
  if (invalidCnSymbol) {
    return `A 股代码「${invalidCnSymbol[1]}」格式不正确，请添加 sh 或 sz 前缀（如 sh601069、sz000858）。`;
  }

  const businessError = xueqiuBusinessError(detail);
  if (businessError) {
    if (businessError.code === 400016) return XUEQIU_COOKIE_EXPIRED_HINT;
    return `雪球行情服务返回错误：${businessError.description || "未知错误"}（错误码：${businessError.code}）`;
  }

  const status = detail.match(/^(?:Xueqiu API error for .+?|Failed to initialize Xueqiu token): HTTP (\d{3})\b/);
  if (status) return `雪球行情服务返回 HTTP ${status[1]}，请稍后重试。`;

  if (detail.startsWith("Failed to parse Xueqiu")) {
    return "雪球返回的行情数据格式异常，请稍后重试。";
  }
  if (/^(?:No data from Xueqiu|No quote data from Xueqiu|Missing (?:stock name|current price) in Xueqiu|Xueqiu realtime response omitted a symbol)/.test(detail)) {
    return "雪球未返回完整的股票行情，请检查股票代码和市场。";
  }
  if (/^(?:Invalid .*symbol .*Xueqiu|Xueqiu realtime symbol normalization failed)/.test(detail)) {
    return "股票代码格式不正确，请检查股票代码和市场。";
  }

  if (
    detail.startsWith("Network error") ||
    detail.startsWith("Failed to initialize Xueqiu token:") ||
    detail.startsWith("Failed to read Xueqiu")
  ) {
    return XUEQIU_API_FAILED_HINT;
  }
  return `雪球行情服务错误：${detail}`;
}
