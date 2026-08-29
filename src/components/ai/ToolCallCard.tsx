import { useMemo, useState, type ReactNode } from "react";
import { Collapse, Tooltip } from "antd";
import {
  CheckCircleFilled,
  CloseCircleFilled,
  DatabaseOutlined,
  LineChartOutlined,
  LoadingOutlined,
  PieChartOutlined,
  SwapOutlined,
  BellOutlined,
  TableOutlined,
  FundOutlined,
} from "@ant-design/icons";
import type { ToolCallInfo } from "../../types";

// Friendly Chinese labels for tool names. Kept in sync with the backend's
// tool definitions (src-tauri/src/services/ai_tools.rs). Exported so the chat
// page can reuse it for the legacy name-only badge fallback.
export const TOOL_LABELS: Record<string, string> = {
  get_market_overview: "大盘总览",
  get_stock_quote: "实时行情",
  get_price_history: "价格历史",
  search_stock: "代码查询",
  get_stock_fundamentals: "估值基本面",
  get_technical_indicators: "技术指标",
  get_financial_statements: "财务报表",
  get_portfolio_overview: "组合总览",
  get_holdings_detail: "持仓明细",
  get_dashboard_summary: "资产总览",
  get_transactions: "交易记录",
  get_performance_metrics: "绩效指标",
  get_return_attribution: "收益归因",
  get_monthly_returns: "月度收益",
  get_drawdown_analysis: "回撤分析",
  get_risk_metrics: "风险指标",
  get_holding_ranking: "持仓排名",
  get_dividend_income: "分红收入",
  check_price_alerts: "价格提醒",
  get_option_positions: "期权持仓",
  get_option_review: "期权操作复盘",
  get_stock_review: "股票操作复盘",
  save_stock_review_annotation: "保存股票复盘注释",
};

/** Pick an icon for a tool based on its category, inferred from the name. */
function toolIcon(name: string) {
  if (name.startsWith("get_market_overview") || name.startsWith("get_stock_quote") || name.startsWith("get_price_history")) {
    return <LineChartOutlined />;
  }
  if (name.startsWith("get_stock_fundamentals") || name.startsWith("get_technical_indicators") || name.startsWith("get_financial_statements")) {
    return <FundOutlined />;
  }
  if (name.startsWith("get_performance") || name.startsWith("get_risk") || name.startsWith("get_drawdown") || name.startsWith("get_holding_ranking")) {
    return <FundOutlined />;
  }
  if (name.startsWith("get_return") || name.startsWith("get_monthly") || name.startsWith("get_dividend")) {
    return <PieChartOutlined />;
  }
  if (name.startsWith("get_transactions")) return <SwapOutlined />;
  if (name.startsWith("check_price_alerts")) return <BellOutlined />;
  if (name.startsWith("search_stock")) return <DatabaseOutlined />;
  if (name.startsWith("get_option")) return <TableOutlined />;
  if (name.startsWith("get_stock_review") || name.startsWith("save_stock_review")) return <TableOutlined />;
  return <DatabaseOutlined />;
}

/** Pretty-print a JSON string, falling back to the raw text if invalid. */
function prettyJson(raw?: string): string {
  if (!raw) return "";
  const trimmed = raw.trim();
  if (!trimmed) return "";
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return raw;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Result rendering: translate the raw English JSON into a Chinese key-value
// view so users don't stare at `pe_ttm` / `total_return`. A global field-label
// map + smart formatters cover most tools; tools with list/nested shapes get
// a bespoke renderer. Anything unmapped falls back to pretty-printed JSON.
// ─────────────────────────────────────────────────────────────────────────────

/** Global English-key → Chinese-label map shared across all tools. */
const FIELD_LABELS: Record<string, string> = {
  // identity
  symbol: "代码",
  name: "名称",
  market: "市场",
  currency: "币种",
  note: "备注",
  // quote / price
  current_price: "现价",
  previous_close: "昨收",
  change: "涨跌额",
  change_percent: "涨跌幅",
  high: "最高",
  low: "最低",
  open: "开盘",
  close: "收盘",
  volume: "成交量",
  amount: "成交额",
  turnover_rate: "换手率",
  updated_at: "更新时间",
  // fundamentals
  pe_ttm: "市盈率(PE-TTM)",
  pb: "市净率(PB)",
  market_cap: "总市值",
  dividend_yield: "股息率",
  eps: "每股收益(EPS)",
  roe: "净资产收益率(ROE)",
  // performance
  start_date: "起始日",
  end_date: "结束日",
  start_value: "期初市值",
  end_value: "期末市值",
  total_return: "累计收益率",
  annualized_return: "年化收益率",
  total_pnl: "累计盈亏",
  total_pnl_percent: "累计盈亏率",
  daily_pnl: "当日盈亏",
  max_drawdown: "最大回撤",
  drawdown_duration_days: "回撤持续(天)",
  recovery_duration_days: "恢复持续(天)",
  peak_date: "峰值日",
  trough_date: "谷底日",
  recovery_date: "恢复日",
  volatility: "波动率",
  daily_volatility: "日波动率",
  annualized_volatility: "年化波动率",
  sharpe_ratio: "夏普比率",
  calmar_ratio: "Calmar比率",
  risk_free_rate: "无风险利率",
  data_points: "数据点数",
  // dashboard
  total_market_value: "总市值",
  total_cost: "总成本",
  us_market_value: "美股市值",
  cn_market_value: "A股市值",
  hk_market_value: "港股市值",
  base_currency: "基础币种",
  // transactions / holdings
  transaction_type: "类型",
  shares: "数量",
  price: "价格",
  total_amount: "金额",
  commission: "手续费",
  traded_at: "成交时间",
  account_name: "账户",
  category: "类别",
  count: "笔数",
  payment_count: "笔数",
  net_income: "净收入",
  total_net_income: "净收入合计",
  // search
  direction: "方向",
  // technical
  latest_close: "最新收盘",
  latest_date: "最新日期",
};

/** Field keys whose numeric value is already a percentage (display with %). */
const PCT_KEYS = new Set([
  "change_percent",
  "total_return",
  "annualized_return",
  "total_pnl_percent",
  "max_drawdown",
  "volatility",
  "daily_volatility",
  "annualized_volatility",
  "turnover_rate",
  "dividend_yield",
  "roe",
  "contribution_percent",
  "revenue_yoy",
  "net_profit_yoy",
  "debt_ratio",
  "return_rate",
  "risk_free_rate",
]);

/** Field keys whose value is a large currency amount shown in 亿. */
const YI_KEYS = new Set([
  "market_cap",
  "total_market_value",
  "total_cost",
  "us_market_value",
  "cn_market_value",
  "hk_market_value",
  "start_value",
  "end_value",
  "total_pnl",
  "daily_pnl",
  "market_value",
  "cost_value",
  "total_amount",
  "net_income",
  "total_net_income",
  "revenue",
  "net_profit",
  "total_assets",
]);

/** Format a number in 亿 (1e8) units, e.g. 1.63e12 → "16314.00 亿". */
function fmtYi(v: number): string {
  return `${(v / 1e8).toFixed(2)} 亿`;
}

/** Format a number that is already a percent (19.82 → "19.82%"). */
function fmtPct(v: number): string {
  return `${v.toFixed(2)}%`;
}

/** Format a plain decimal, trailing-zero trimmed to 4 places. */
function fmtNum(v: number): string {
  return Number.isInteger(v) ? `${v}` : `${Number(v.toFixed(4))}`;
}

/** Format a value for a known field key, applying %/亿/number conventions. */
function fmtField(key: string, value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "number") {
    if (!Number.isFinite(value)) return "—";
    if (PCT_KEYS.has(key)) return fmtPct(value);
    if (YI_KEYS.has(key) && Math.abs(value) >= 1e6) return fmtYi(value);
    return fmtNum(value);
  }
  return String(value);
}

/** One labelled value row in a result panel. Hides null/empty values. */
function Field({ label, value }: { label: string; value: ReactNode }) {
  if (value === null || value === undefined || value === "") return null;
  return (
    <div className="ai-tool-field">
      <span className="ai-tool-field-label">{label}</span>
      <span className="ai-tool-field-value">{value}</span>
    </div>
  );
}

/** A small section heading inside a result panel. */
function Group({ children }: { children: ReactNode }) {
  return <div className="ai-tool-field-group">{children}</div>;
}

/**
 * Generic renderer: for a flat object, show each known field as a Chinese
 * labelled row (skipping unknown / nested fields). Used as the base for most
 * tools and as the body of bespoke renderers.
 */
function FieldsList({
  data,
  order,
}: {
  data: Record<string, unknown>;
  order?: string[];
}) {
  const keys = order ?? Object.keys(data);
  return (
    <div className="ai-tool-fields">
      {keys.map((k) => {
        const v = data[k];
        if (v === null || v === undefined || v === "") return null;
        const label = FIELD_LABELS[k];
        if (!label) return null; // skip unmapped / nested keys
        if (Array.isArray(v) || (typeof v === "object" && v !== null)) return null;
        return <Field key={k} label={label} value={fmtField(k, v)} />;
      })}
    </div>
  );
}

/** Pretty-printed raw JSON fallback. */
function RawJson({ json }: { json: string }) {
  return <pre className="ai-tool-card-code">{prettyJson(json)}</pre>;
}

// ── Per-tool renderers (bespoke shapes only) ─────────────────────────────────

/** A single real-time quote (StockQuote flat object). */
function QuoteResult({ data }: { data: Record<string, unknown> }) {
  const n = (k: string) => (data[k] == null ? null : Number(data[k]));
  return (
    <div className="ai-tool-fields">
      <Field label="股票" value={`${data.name ?? ""}（${data.symbol ?? ""}）`} />
      <Field label="现价" value={n("current_price") != null ? fmtNum(n("current_price")!) : null} />
      <Field label="涨跌" value={n("change") != null ? `${fmtNum(n("change")!)} (${fmtPct(n("change_percent") ?? 0)})` : null} />
      <Field label="昨收" value={n("previous_close") != null ? fmtNum(n("previous_close")!) : null} />
      <Field label="最高 / 最低" value={n("high") != null ? `${fmtNum(n("high")!)} / ${fmtNum(n("low")!)}` : null} />
      <Field label="成交量" value={n("volume") != null ? fmtNum(n("volume")!) : null} />
    </div>
  );
}

/** Name ↔ symbol lookup. */
function SearchResult({ data }: { data: Record<string, unknown> }) {
  return (
    <div className="ai-tool-fields">
      <Field label="名称" value={data.name != null ? String(data.name) : null} />
      <Field label="代码" value={data.symbol != null ? String(data.symbol) : "未找到"} />
      {data.market ? <Field label="市场" value={String(data.market)} /> : null}
      {data.note ? <Field label="备注" value={String(data.note)} /> : null}
    </div>
  );
}

/** Price-history series as a compact first/last/return summary + recent rows. */
function PriceHistoryResult({ data }: { data: Record<string, unknown> }) {
  const points = Array.isArray(data.points) ? (data.points as Record<string, unknown>[]) : [];
  if (points.length === 0) return <RawJson json={JSON.stringify(data)} />;
  const first = Number(points[0].close);
  const last = Number(points[points.length - 1].close);
  const ret = first !== 0 ? ((last - first) / first) * 100 : 0;
  return (
    <div className="ai-tool-fields">
      <Field label="标的" value={`${data.symbol ?? ""}（${data.market ?? ""}）`} />
      <Field label="数据点数" value={`${points.length} 个交易日`} />
      <Field label="区间收益" value={`${fmtNum(last)} vs ${fmtNum(first)}（${fmtPct(ret)}）`} />
      <Group>近 {Math.min(points.length, 5)} 日</Group>
      {points.slice(-5).reverse().map((p, i) => (
        <Field key={i} label={String(p.date ?? "")} value={fmtNum(Number(p.close))} />
      ))}
    </div>
  );
}

/** Technical indicators (grouped). */
function TechnicalResult({ data }: { data: Record<string, unknown> }) {
  const n = (k: string) => (data[k] == null ? null : Number(data[k]));
  return (
    <div className="ai-tool-fields">
      <Field label="标的" value={`${data.symbol ?? ""}（${data.market ?? ""}）`} />
      <Field label="最新日期" value={data.latest_date != null ? String(data.latest_date) : null} />
      <Field label="收盘价" value={n("latest_close") != null ? fmtNum(n("latest_close")!) : null} />
      <Field label="数据点数" value={data.data_points != null ? String(data.data_points) : null} />
      <Group>均线</Group>
      <Field label="MA5" value={n("ma5") != null ? fmtNum(n("ma5")!) : "—"} />
      <Field label="MA10" value={n("ma10") != null ? fmtNum(n("ma10")!) : "—"} />
      <Field label="MA20" value={n("ma20") != null ? fmtNum(n("ma20")!) : "—"} />
      <Field label="MA60" value={n("ma60") != null ? fmtNum(n("ma60")!) : "—"} />
      <Group>MACD(12,26,9)</Group>
      <Field label="DIF" value={n("macd_dif") != null ? fmtNum(n("macd_dif")!) : "—"} />
      <Field label="DEA" value={n("macd_dea") != null ? fmtNum(n("macd_dea")!) : "—"} />
      <Field label="柱状" value={n("macd_histogram") != null ? fmtNum(n("macd_histogram")!) : "—"} />
      <Group>其他</Group>
      <Field label="RSI(14)" value={n("rsi14") != null ? fmtNum(n("rsi14")!) : "—"} />
      <Field label="布林上轨" value={n("bollinger_upper") != null ? fmtNum(n("bollinger_upper")!) : "—"} />
      <Field label="布林中轨" value={n("bollinger_middle") != null ? fmtNum(n("bollinger_middle")!) : "—"} />
      <Field label="布林下轨" value={n("bollinger_lower") != null ? fmtNum(n("bollinger_lower")!) : "—"} />
    </div>
  );
}

/** Fundamentals (估值). */
function FundamentalsResult({ data }: { data: Record<string, unknown> }) {
  const n = (k: string) => (data[k] == null ? null : Number(data[k]));
  return (
    <div className="ai-tool-fields">
      <Field label="股票" value={`${data.name ?? ""}（${data.symbol ?? ""}）`} />
      <Field label="现价" value={n("current_price") != null ? fmtNum(n("current_price")!) : null} />
      <Field label="市盈率(PE-TTM)" value={n("pe_ttm") != null ? fmtNum(n("pe_ttm")!) : "—"} />
      <Field label="市净率(PB)" value={n("pb") != null ? fmtNum(n("pb")!) : "—"} />
      <Field label="总市值" value={n("market_cap") != null ? fmtYi(n("market_cap")!) : null} />
      <Field label="股息率" value={n("dividend_yield") != null ? fmtPct(n("dividend_yield")!) : "—"} />
      <Field label="每股收益(EPS)" value={n("eps") != null ? fmtNum(n("eps")!) : "—"} />
      <Field label="净资产收益率(ROE)" value={n("roe") != null ? fmtPct(n("roe")!) : "—"} />
      <Field label="换手率" value={n("turnover_rate") != null ? fmtPct(n("turnover_rate")!) : "—"} />
    </div>
  );
}

/** Financial statements (multi-period). */
function FinancialResult({ data }: { data: Record<string, unknown> }) {
  const periods = Array.isArray(data.periods) ? (data.periods as Record<string, unknown>[]) : [];
  if (periods.length === 0) {
    return <div className="ai-tool-note">{(data.note as string) ?? "无数据"}</div>;
  }
  const n = (row: Record<string, unknown>, k: string) => (row[k] == null ? null : Number(row[k]));
  return (
    <div className="ai-tool-fields">
      <Field label="标的" value={`${data.symbol ?? ""}（${data.market ?? ""} A 股）`} />
      {periods.map((p, i) => (
        <div key={i} className="ai-tool-field-period">
          <Group>{String(p.period_name ?? p.report_date ?? `第 ${i + 1} 期`)}</Group>
          <Field label="每股收益(EPS)" value={n(p, "eps") != null ? fmtNum(n(p, "eps")!) : "—"} />
          <Field label="净资产收益率(ROE)" value={n(p, "roe") != null ? fmtPct(n(p, "roe")!) : "—"} />
          <Field label="营业收入" value={n(p, "revenue") != null ? fmtYi(n(p, "revenue")!) : "—"} />
          <Field label="营收同比" value={n(p, "revenue_yoy") != null ? fmtPct(n(p, "revenue_yoy")!) : "—"} />
          <Field label="净利润" value={n(p, "net_profit") != null ? fmtYi(n(p, "net_profit")!) : "—"} />
          <Field label="净利同比" value={n(p, "net_profit_yoy") != null ? fmtPct(n(p, "net_profit_yoy")!) : "—"} />
          <Field label="资产负债率" value={n(p, "debt_ratio") != null ? fmtPct(n(p, "debt_ratio")!) : "—"} />
        </div>
      ))}
    </div>
  );
}

/** Transaction rows as a compact list. */
function TransactionsResult({ data }: { data: Record<string, unknown> }) {
  const txns = Array.isArray(data.transactions) ? (data.transactions as Record<string, unknown>[]) : [];
  if (txns.length === 0) return <div className="ai-tool-note">无交易记录</div>;
  return (
    <div className="ai-tool-fields">
      <Field label="记录数" value={String(data.count ?? txns.length)} />
      {txns.slice(0, 10).map((t, i) => (
        <div key={i} className="ai-tool-field-period">
          <Group>{String(t.traded_at ?? "")}</Group>
          <Field label="标的" value={`${t.name ?? ""}（${t.symbol ?? ""}）`} />
          <Field label="类型" value={String(t.transaction_type ?? "")} />
          <Field label="数量 / 价格" value={`${fmtNum(Number(t.shares ?? 0))} @ ${fmtNum(Number(t.price ?? 0))}`} />
          <Field label="金额" value={fmtField("total_amount", t.total_amount)} />
        </div>
      ))}
      {txns.length > 10 ? <div className="ai-tool-note">…共 {txns.length} 条，仅显示前 10 条</div> : null}
    </div>
  );
}

/** Dividend income summary. */
function DividendResult({ data }: { data: Record<string, unknown> }) {
  const rows = Array.isArray(data.by_symbol) ? (data.by_symbol as Record<string, unknown>[]) : [];
  return (
    <div className="ai-tool-fields">
      <Field label="净收入合计" value={fmtField("total_net_income", data.total_net_income)} />
      <Field label="笔数" value={String(data.payment_count ?? "")} />
      <Group>按标的</Group>
      {rows.map((r, i) => (
        <Field
          key={i}
          label={`${r.name ?? r.symbol ?? ""}（${r.currency ?? ""}）`}
          value={`${fmtField("net_income", r.net_income)} × ${r.count ?? 0}`}
        />
      ))}
    </div>
  );
}

/**
 * Pick the right renderer for a tool's result JSON. Flat objects with a known
 * shape use a bespoke renderer; otherwise the generic FieldsList / RawJson
 * fallback keeps things readable without losing data.
 */
function ToolResultView({ name, resultJson }: { name: string; resultJson: string }) {
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = JSON.parse(resultJson) as Record<string, unknown>;
  } catch {
    parsed = null;
  }
  if (!parsed || typeof parsed !== "object") {
    return <RawJson json={resultJson} />;
  }
  // Bespoke renderers for tools with a clear flat / list shape.
  switch (name) {
    case "get_stock_quote":
      return <QuoteResult data={parsed} />;
    case "search_stock":
      return <SearchResult data={parsed} />;
    case "get_price_history":
      return <PriceHistoryResult data={parsed} />;
    case "get_stock_fundamentals":
      return <FundamentalsResult data={parsed} />;
    case "get_technical_indicators":
      return <TechnicalResult data={parsed} />;
    case "get_financial_statements":
      return <FinancialResult data={parsed} />;
    case "get_transactions":
      return <TransactionsResult data={parsed} />;
    case "get_dividend_income":
      return <DividendResult data={parsed} />;
    default:
      break;
  }
  // Generic path: if the object has at least one mapped flat field, render as
  // labelled rows; otherwise fall back to raw JSON (handles nested/list tools
  // like portfolio overview, holdings detail, return attribution, alerts,
  // options where a bespoke layout adds little over the model's prose).
  const keys = Object.keys(parsed);
  const hasMappedFlat = keys.some(
    (k) =>
      FIELD_LABELS[k] &&
      !Array.isArray(parsed[k]) &&
      !(typeof parsed[k] === "object" && parsed[k] !== null),
  );
  if (hasMappedFlat) {
    return <FieldsList data={parsed} />;
  }
  return <RawJson json={resultJson} />;
}

function StatusBadge({ status }: { status: ToolCallInfo["status"] }) {
  if (status === "running") {
    return (
      <Tooltip title="执行中">
        <LoadingOutlined style={{ color: "var(--color-info)" }} />
      </Tooltip>
    );
  }
  if (status === "success") {
    return (
      <Tooltip title="成功">
        <CheckCircleFilled style={{ color: "var(--color-success)" }} />
      </Tooltip>
    );
  }
  return (
    <Tooltip title="失败">
      <CloseCircleFilled style={{ color: "var(--color-error)" }} />
    </Tooltip>
  );
}

/**
 * One tool invocation rendered as a Claude-style expandable card.
 *
 * Collapsed (default): a single line with icon, friendly name, status, and
 * duration. Expanded: the arguments and (on success) the returned data, each
 * as pretty-printed JSON in a scrollable code block. The user opts in to
 * expand — the collapsed row carries enough signal (status spinner/check, name,
 * duration) for the common scan-the-conversation case.
 */
export function ToolCallCard({ tool }: { tool: ToolCallInfo }) {
  // Always start collapsed — the collapsed row (icon + name + status + duration)
  // is enough signal at a glance; the user expands to inspect args/results.
  const [expanded, setExpanded] = useState(false);
  const label = TOOL_LABELS[tool.name] ?? tool.name;

  const argsPretty = useMemo(() => prettyJson(tool.arguments), [tool.arguments]);
  const resultPretty = useMemo(() => prettyJson(tool.result), [tool.result]);

  const durationLabel =
    typeof tool.durationMs === "number"
      ? tool.durationMs < 1000
        ? `${tool.durationMs} ms`
        : `${(tool.durationMs / 1000).toFixed(1)} s`
      : null;

  return (
    <div className="ai-tool-card">
      <Collapse
        size="small"
        // Default collapsed; the user opts in to inspect args/results.
        activeKey={expanded ? ["1"] : []}
        onChange={(key) => setExpanded(Array.isArray(key) && key.length > 0)}
        className="ai-tool-card-collapse"
        items={[
          {
            key: "1",
            label: (
              <div className="flex items-center gap-2 w-full min-w-0">
                <span className="flex-shrink-0" style={{ color: "var(--color-text-secondary)" }}>{toolIcon(tool.name)}</span>
                <span className="truncate" style={{ color: "var(--color-text)" }}>{label}</span>
                <span className="flex-shrink-0 ml-auto flex items-center gap-2">
                  {durationLabel && (
                    <span className="text-xs" style={{ color: "var(--color-text-tertiary)" }}>{durationLabel}</span>
                  )}
                  <StatusBadge status={tool.status} />
                </span>
              </div>
            ),
            children: (
              <div className="space-y-2">
                {tool.status === "error" && tool.error ? (
                  <div className="text-sm whitespace-pre-wrap break-words" style={{ color: "var(--color-error)" }}>
                    {tool.error}
                  </div>
                ) : null}
                {argsPretty && (
                  <div>
                    <div className="text-xs mb-0.5" style={{ color: "var(--color-text-tertiary)" }}>入参</div>
                    <pre className="ai-tool-card-code">{argsPretty}</pre>
                  </div>
                )}
                {tool.status === "running" && !resultPretty && (
                  <div className="text-sm flex items-center gap-1.5" style={{ color: "var(--color-text-tertiary)" }}>
                    <LoadingOutlined /> 正在执行…
                  </div>
                )}
                {resultPretty && (
                  <div>
                    <div className="text-xs mb-0.5" style={{ color: "var(--color-text-tertiary)" }}>返回结果</div>
                    <ToolResultView name={tool.name} resultJson={tool.result ?? ""} />
                  </div>
                )}
              </div>
            ),
          },
        ]}
      />
    </div>
  );
}

/** Renders a list of tool calls as stacked cards. */
export function ToolCallList({ tools }: { tools: ToolCallInfo[] }) {
  if (!tools || tools.length === 0) return null;
  return (
    <div className="flex flex-col gap-1 mb-1.5">
      {tools.map((t) => (
        <ToolCallCard key={t.id} tool={t} />
      ))}
    </div>
  );
}
