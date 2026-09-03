//! AI tools: lets the assistant call back into the app for real data.
//!
//! The chat loop (`ai_chat_service::chat_stream`) advertises these tools to the
//! LLM via the OpenAI-style `tools` field. When the model returns a
//! `tool_calls` finish reason, `execute_tool` runs the matching handler — which
//! reuses existing in-process services (quote fetch, holdings, performance) —
//! and the JSON result is fed back to the model as a `tool`-role message.
//!
//! This is the counterpart to the Markdown "skills" system: skills inject
//! *instructions* into the prompt; tools let the model *fetch data* on demand.
//! A skill says "answer in this structure"; a tool says "here are real numbers".

use crate::commands::ocr::{
    lookup_cn_stock_code_with_state, lookup_stock_name_by_symbol_with_state,
};
use crate::commands::options::get_option_contracts_inner;
use crate::commands::transactions::query_transactions_inner;
use crate::db::Database;
use crate::models::dashboard::DashboardSummary;
use crate::models::option_review::OptionReviewReport;
use crate::models::quote::ExchangeRates;
use crate::models::stock_operation_review::StockOperationReviewQuery;
use crate::services::ai_chat_service::build_portfolio_context;
use crate::services::alert_service;
use crate::services::exchange_rate_service::{
    convert_currency, get_cached_rates, ExchangeRateCache,
};
use crate::services::indicators;
use crate::services::market_overview_service;
use crate::services::option_review_service;
use crate::services::performance_service::{self, PerformanceFilter};
use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
use crate::services::quote_provider_service;
use crate::services::quote_service::{self, resolve_index_secid, QuoteCache, QuoteServiceState};
use crate::services::stock_operation_builder::normalize_stock_symbol;
use crate::services::stock_operation_review_service;
use chrono::{Duration, NaiveDate, Utc};
use serde_json::{json, Value};

/// Maximum number of sequential tool rounds in a single chat turn. Each round
/// may execute several tool calls in parallel (the model often batches them),
/// but we cap the number of *rounds* to avoid an infinite ping-pong between
/// the model and the app. Five is generous: real conversations need 1–2.
pub const MAX_TOOL_ROUNDS: usize = 1000;

// ─────────────────────────────────────────────────────────────────────────────
// Tool definitions (OpenAI function-calling schema)
// ─────────────────────────────────────────────────────────────────────────────

/// The `tools` array sent in the `/chat/completions` request body. Each entry
/// is an OpenAI "function" tool: a name, a human-readable description (the
/// model uses this to decide when to call it), and a JSON-Schema `parameters`
/// block describing the arguments.
///
/// Descriptions are intentionally detailed and written for the model — they are
/// the only signal the model has about *when* each tool is useful.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "get_market_overview",
                "description": "获取今日主要市场指数行情与用户持仓当日表现。适用于用户询问\"今天大盘怎么样\"\"股市今天如何\"\"市场表现\"等关于整体行情的问题。返回主要指数（标普500、纳指、道指、恒生、沪深300、上证）的现价/涨跌幅，以及用户当前持仓的当日合计盈亏。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_stock_quote",
                "description": "获取某只股票的实时行情（现价、涨跌额、涨跌幅、最高、最低、成交量）。当用户询问某只具体股票的当前价格或当日表现时调用。symbol 为股票代码；market 可选，不提供时根据代码格式自动推断。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "股票代码，例如 \"AAPL\"、\"0700.HK\"、\"SH600519\""
                        },
                        "market": {
                            "type": "string",
                            "enum": ["US", "HK", "CN"],
                            "description": "市场：US 美股 / HK 港股 / CN A股。可选，不填时按代码格式推断。"
                        }
                    },
                    "required": ["symbol"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_price_history",
                "description": "获取某只股票近 N 个交易日的收盘价序列（默认30天）。适用于用户询问近期走势、价格历史、是否创新高/新低等问题。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "股票代码"
                        },
                        "market": {
                            "type": "string",
                            "enum": ["US", "HK", "CN"],
                            "description": "市场，可选，不填时按代码格式推断。"
                        },
                        "days": {
                            "type": "integer",
                            "description": "回溯的交易日天数，默认 30，最大 365",
                            "default": 30
                        }
                    },
                    "required": ["symbol"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_portfolio_overview",
                "description": "获取用户当前投资组合的结构化总览（账户总览、持仓表、近期交易、绩效指标）。当用户询问\"我的持仓\"\"组合表现\"\"整体盈亏\"时调用。注意：这与对话自动注入的快照内容相同，仅在用户关闭了自动注入或需要确认最新数据时调用。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_holdings_detail",
                "description": "获取用户当前持仓的明细列表（每只持仓的代码、名称、市场、持仓量、均价、现价、市值、盈亏等）。当需要逐只持仓分析或排序时调用。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_performance_metrics",
                "description": "获取用户组合在指定区间的绩效指标（累计/年化收益率、最大回撤、波动率、夏普比率、收益序列）。当用户询问收益、夏普、回撤、波动率等绩效问题时调用。periodDays 指回溯天数，默认 365（近1年）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "periodDays": {
                            "type": "integer",
                            "description": "回溯天数，默认 365，最大 3650",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "search_stock",
                "description": "根据名称搜索股票代码，或根据代码查询股票名称。当用户用中文名称提到一只你不确定代码的股票时（例如\"茅台\"\"腾讯\"），先调用此工具解析出代码，再调用 get_stock_quote 等行情工具。direction 为 name_to_symbol（名称查代码，仅支持 A 股名称）或 symbol_to_name（代码查名称，支持所有市场）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "查询内容：名称（如\"茅台\"）或代码（如\"AAPL\"、\"0700.HK\"）"
                        },
                        "direction": {
                            "type": "string",
                            "enum": ["name_to_symbol", "symbol_to_name"],
                            "default": "name_to_symbol"
                        }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_transactions",
                "description": "查询用户的交易记录，支持按类型、日期、标的过滤。适用于用户询问\"最近买了什么\"\"卖出记录\"\"分红\"\"近期交易\"等。默认返回最近 50 条；txType 可指定 BUY/SELL/OPEN/PAY（分红）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "txType": {
                            "type": "string",
                            "enum": ["BUY", "SELL", "OPEN", "PAY"],
                            "description": "交易类型过滤，可选。PAY 为分红/利息。"
                        },
                        "symbol": {
                            "type": "string",
                            "description": "按股票代码过滤，可选"
                        },
                        "days": {
                            "type": "integer",
                            "description": "只查最近 N 天的交易，可选"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "返回条数上限，默认 50，最大 200",
                            "default": 50
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_return_attribution",
                "description": "收益归因分析：把组合盈亏拆解到各市场、类别、个股，看谁贡献了收益/亏损。适用于\"收益主要来自哪\"\"哪些标的赚/亏得最多\"等问题。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "periodDays": {
                            "type": "integer",
                            "description": "回溯天数，默认 365",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_monthly_returns",
                "description": "按月统计收益序列（每月的收益率、盈亏、期初期末市值）。适用于\"月度收益\"\"哪几个月赚了/亏了\"\"收益分布\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "periodDays": {
                            "type": "integer",
                            "description": "回溯天数，默认 365",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_drawdown_analysis",
                "description": "最大回撤分析：回撤幅度、峰值/谷底日期、恢复日期、回撤持续天数。适用于\"最大回撤\"\"最惨的时候跌了多少\"\"多久恢复\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "periodDays": {
                            "type": "integer",
                            "description": "回溯天数，默认 365",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_risk_metrics",
                "description": "风险指标：日/年化波动率、夏普比率、最大回撤、Calmar 比率。适用于\"风险大不大\"\"波动率\"\"夏普\"\"Calmar\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "periodDays": {
                            "type": "integer",
                            "description": "回溯天数，默认 365",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_holding_ranking",
                "description": "持仓绩效排名：按收益或盈亏对个股排序。适用于\"哪只股票表现最好/最差\"\"收益排名\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sortBy": {
                            "type": "string",
                            "enum": ["pnl", "return_rate"],
                            "description": "排序字段：pnl 盈亏金额 / return_rate 收益率",
                            "default": "pnl"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "返回条数，默认 10",
                            "default": 10
                        },
                        "periodDays": {
                            "type": "integer",
                            "description": "回溯天数，默认 365",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_dashboard_summary",
                "description": "组合仪表盘总览：总市值/总成本/总盈亏/当日盈亏，并按市场（美股/港股/A股）拆分。适用于\"总资产多少\"\"各市场分布\"\"整体盈亏\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_dividend_income",
                "description": "分红/利息收入汇总：按标的聚合 PAY 类型交易的净收入（金额 - 手续费），并给出合计。适用于\"分红多少\"\"收了多少利息\"\"被动收入\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "days": {
                            "type": "integer",
                            "description": "只统计最近 N 天的分红，可选；不填则统计全部",
                            "default": 365
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "check_price_alerts",
                "description": "检查用户设置的价格提醒是否已触发（基于缓存行情）。适用于\"我的提醒触发了吗\"\"到价提醒\"\"关注的价格\"等。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_option_positions",
                "description": "查询期权持仓（卖出期权记录）：按账户列出期权合约，含标的、行权价、到期日、类型（看涨/看跌）、收取权利金、状态（活跃/到期/被行权/平仓）。适用于\"期权持仓\"\"卖了多少期权\"\"到期日\"等。需要 accountId。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "accountId": {
                            "type": "string",
                            "description": "账户 ID"
                        }
                    },
                    "required": ["accountId"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_option_review",
                "description": "确定性期权历史复盘：按账户和可选个股返回Campaign；gross_premium/net_premium_pnl是含进行中Campaign的累计现金口径，completed_gross_premium/completed_net_premium_pnl及留存率、担保名义资本年化收益率、最差Campaign是已完成口径。用于评价CSP/Covered Call哪些做得好、哪些需要改进；不包含未平仓市值盈亏，也不要用于当前到期风险。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "accountId": { "type": "string", "description": "账户 ID" },
                        "symbol": { "type": "string", "description": "可选标的，例如 AAPL" },
                        "periodDays": { "type": "integer", "minimum": 1, "maximum": 3650, "default": 365 },
                        "allHistory": {
                            "type": "boolean",
                            "description": "为 true 时返回全部历史并覆盖 periodDays；省略或为 false 时使用 periodDays（默认 365）",
                            "default": false
                        }
                    },
                    "required": ["accountId"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_stock_review",
                "description": "读取与股票操作复盘页面相同的轻量确定性报告：评价建仓、加仓、减仓和清仓截至复盘期末的价格效果、估算仓位变化及相对所属市场宽基的方向调整效果。只解释返回数值和事实标签，不重算指标，也不把事后涨跌直接判定为决策对错。symbol 可裁剪到单只股票。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "start_date": { "type": "string", "description": "开始日期，YYYY-MM-DD" },
                        "end_date": { "type": "string", "description": "结束日期，YYYY-MM-DD" },
                        "base_currency": { "type": "string", "enum": ["USD", "CNY", "HKD"] },
                        "account_id": { "type": "string", "description": "可选账户 ID" },
                        "market": { "type": "string", "enum": ["US", "CN", "HK"] },
                        "symbol": { "type": "string", "description": "可选股票代码，仅保留该股票的操作并重新汇总" }
                    },
                    "required": ["start_date", "end_date", "base_currency"],
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_stock_fundamentals",
                "description": "获取某只股票的估值与基本面指标：市盈率(PE-TTM)、市净率(PB)、总市值、股息率、每股收益(EPS)、净资产收益率(ROE)、换手率。当用户询问\"这只股票贵不贵\"\"估值\"\"市盈率\"\"市净率\"\"市值\"\"分红\"等估值/基本面问题时调用，是做投资价值分析的关键数据之一。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "股票代码，例如 \"SH600519\"、\"AAPL\"、\"0700.HK\""
                        },
                        "market": {
                            "type": "string",
                            "enum": ["US", "HK", "CN"],
                            "description": "市场：US 美股 / HK 港股 / CN A股。可选，不填时按代码格式推断。"
                        }
                    },
                    "required": ["symbol"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_technical_indicators",
                "description": "获取某只股票的技术面指标：均线(MA5/MA10/MA20/MA60)、MACD(DIF/DEA/柱)、RSI(14)、布林带(上中下轨)。当用户询问\"技术面\"\"均线\"\"MACD\"\"RSI\"\"超买超卖\"\"压力位支撑位\"\"趋势\"等问题时调用。默认基于近 120 个交易日数据。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "股票代码，例如 \"SH600519\"、\"AAPL\"、\"0700.HK\""
                        },
                        "market": {
                            "type": "string",
                            "enum": ["US", "HK", "CN"],
                            "description": "市场：US 美股 / HK 港股 / CN A股。可选，不填时按代码格式推断。"
                        },
                        "days": {
                            "type": "integer",
                            "description": "用于计算指标的交易日天数，默认 120，最大 365。天数越多均线越完整。"
                        }
                    },
                    "required": ["symbol"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_financial_statements",
                "description": "获取某只 A 股股票近几期财务报表数据：营收、净利润、每股收益(EPS)、净资产收益率(ROE)、资产负债率及同比增速(单位为百分点，如 6.34 表示 +6.34%)。当用户询问\"财务报表\"\"营收\"\"净利润\"\"ROE\"\"负债率\"\"业绩增长\"等基本面财务问题时调用。仅支持 A 股。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "A股股票代码，例如 \"SH600519\"、\"sz000001\""
                        },
                        "periods": {
                            "type": "integer",
                            "description": "要获取的财报期数，默认 4，最大 8。"
                        }
                    },
                    "required": ["symbol"]
                }
            }
        }),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool dispatch
// ─────────────────────────────────────────────────────────────────────────────

/// Arguments shared by every tool handler. All of them are references because
/// tools are short-lived calls inside a single chat turn; they never own state.
pub struct ToolCtx<'a> {
    pub db: &'a Database,
    pub cache: &'a ExchangeRateCache,
    pub quote_cache: &'a QuoteCache,
    pub quote_state: &'a QuoteServiceState,
}

impl<'a> ToolCtx<'a> {
    /// Construct the read-only tool context for an untrusted model turn.
    pub(crate) fn for_untrusted_model_turn(
        db: &'a Database,
        cache: &'a ExchangeRateCache,
        quote_cache: &'a QuoteCache,
        quote_state: &'a QuoteServiceState,
        _user_turn: &str,
    ) -> Self {
        Self {
            db,
            cache,
            quote_cache,
            quote_state,
        }
    }
}

/// Result of a single tool call, ready to be serialized into the `tool`-role
/// message content sent back to the model.
#[derive(Debug)]
pub struct ToolResult {
    /// The JSON content to hand back to the model (always a string in the
    /// OpenAI wire format, so we serialize here).
    pub content: String,
    /// Whether execution succeeded. We currently always return success-shaped
    /// JSON (with an `error` field on failure) so the model can read the error
    /// and recover gracefully; this flag is reserved for future telemetry.
    #[allow(dead_code)]
    pub ok: bool,
}

impl ToolResult {
    fn ok_json(value: Value) -> Self {
        ToolResult {
            content: value.to_string(),
            ok: true,
        }
    }

    fn err_json(message: impl Into<String>) -> Self {
        ToolResult {
            content: json!({ "error": message.into() }).to_string(),
            ok: false,
        }
    }
}

/// Execute a tool call by name. `arguments` is the raw JSON string the model
/// produced (may be empty for no-arg tools). Unknown tool names return an
/// error JSON so the model can apologise rather than the chat hanging.
pub async fn execute_tool(ctx: &ToolCtx<'_>, name: &str, arguments: &str) -> ToolResult {
    let args: Value = if arguments.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err_json(format!("参数解析失败：{e}")),
        }
    };
    match name {
        "get_market_overview" => tool_market_overview(ctx).await,
        "get_stock_quote" => tool_stock_quote(ctx, &args).await,
        "get_price_history" => tool_price_history(ctx, &args).await,
        "get_portfolio_overview" => tool_portfolio_overview(ctx).await,
        "get_holdings_detail" => tool_holdings_detail(ctx).await,
        "get_performance_metrics" => tool_performance_metrics(ctx, &args).await,
        "search_stock" => tool_search_stock(ctx, &args).await,
        "get_transactions" => tool_transactions(ctx, &args).await,
        "get_return_attribution" => tool_return_attribution(ctx, &args).await,
        "get_monthly_returns" => tool_monthly_returns(ctx, &args).await,
        "get_drawdown_analysis" => tool_drawdown_analysis(ctx, &args).await,
        "get_risk_metrics" => tool_risk_metrics(ctx, &args).await,
        "get_holding_ranking" => tool_holding_ranking(ctx, &args).await,
        "get_dashboard_summary" => tool_dashboard_summary(ctx).await,
        "get_dividend_income" => tool_dividend_income(ctx, &args).await,
        "check_price_alerts" => tool_check_alerts(ctx).await,
        "get_option_positions" => tool_option_positions(ctx, &args).await,
        "get_option_review" => tool_option_review(ctx, &args).await,
        "get_stock_review" => tool_stock_review(ctx, &args).await,
        "get_stock_fundamentals" => tool_stock_fundamentals(ctx, &args).await,
        "get_technical_indicators" => tool_technical_indicators(ctx, &args).await,
        "get_financial_statements" => tool_financial_statements(ctx, &args).await,
        other => ToolResult::err_json(format!("未知工具：{other}")),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-tool handlers
// ─────────────────────────────────────────────────────────────────────────────

async fn tool_market_overview(ctx: &ToolCtx<'_>) -> ToolResult {
    match market_overview_service::get_market_overview(ctx.db, ctx.cache, ctx.quote_cache).await {
        Ok(overview) => ToolResult::ok_json(serde_json::to_value(&overview).unwrap_or(json!({}))),
        Err(e) => ToolResult::err_json(format!("获取大盘总览失败：{e}")),
    }
}

/// Infer a market from a symbol's format when the model omits it. Mirrors the
/// conventions used across the app (HK = `NNNN.HK`, CN A-share = `SH/`SZ` prefix
/// or 6-digit code). Falls back to US.
fn infer_market(symbol: &str) -> &'static str {
    let s = symbol.trim();
    if s.ends_with(".HK") || s.ends_with(".SS") {
        "HK"
    } else if s.starts_with("SH") || s.starts_with("SZ") || s.starts_with("BJ") {
        "CN"
    } else if s.len() == 5 && s.chars().all(|c| c.is_ascii_digit()) {
        // Bare 5-digit codes are HK (e.g. "00700").
        "HK"
    } else if s.len() == 6 && s.chars().all(|c| c.is_ascii_digit()) {
        "CN"
    } else {
        "US"
    }
}

async fn tool_stock_quote(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let symbol = match args.get("symbol").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => return ToolResult::err_json("缺少参数 symbol"),
    };
    if symbol.is_empty() {
        return ToolResult::err_json("symbol 不能为空");
    }

    // Index symbols (^GSPC, HSI, 000300.SS, …) are NOT recognised by the stock
    // fetchers (Yahoo 403s them, xueqiu/eastmoney stock endpoints return
    // errors). Route them to the EastMoney index endpoint instead, which needs
    // no auth and covers every major index. This is the fix for the
    // "获取 ^GSPC 行情失败：HTTP 403" class of errors.
    if let Some((secid, name)) = resolve_index_secid(&symbol) {
        let market = if secid.starts_with("1.") || secid.starts_with("0.") {
            "CN"
        } else if secid == "100.HSI" {
            "HK"
        } else {
            "US"
        };
        if let Some(cached) = ctx.quote_cache.get(&symbol) {
            return ToolResult::ok_json(json!(cached));
        }
        return match quote_service::fetch_index_quote_eastmoney(secid, &symbol, market).await {
            Ok(q) => {
                ctx.quote_cache.set(q.clone());
                ToolResult::ok_json(json!({ "quote": q, "index_name": name }))
            }
            Err(e) => ToolResult::err_json(format!("获取指数 {symbol}（{name}）行情失败：{e}")),
        };
    }

    let market = args
        .get("market")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_uppercase())
        .unwrap_or_else(|| infer_market(&symbol).to_string());

    // Serve from cache first (fast, offline-friendly); only hit the network on
    // a miss, exactly like the holding-quote command does.
    if let Some(cached) = ctx.quote_cache.get(&symbol) {
        return ToolResult::ok_json(json!(cached));
    }
    let config = match quote_provider_service::get_quote_provider_config(ctx.db) {
        Ok(c) => c,
        Err(e) => return ToolResult::err_json(format!("读取行情源配置失败：{e}")),
    };
    let quote = match market.as_str() {
        "HK" => {
            quote_service::fetch_hk_quote_with_provider(
                ctx.quote_state,
                &symbol,
                &config.hk_provider,
            )
            .await
        }
        "CN" => {
            quote_service::fetch_cn_quote_with_provider(
                ctx.quote_state,
                &symbol,
                &config.cn_provider,
            )
            .await
        }
        _ => {
            quote_service::fetch_us_quote_with_provider(
                ctx.quote_state,
                &symbol,
                &config.us_provider,
            )
            .await
        }
    };
    match quote {
        Ok(result) => {
            ctx.quote_cache.set(result.data.clone());
            ToolResult::ok_json(json!(result.data))
        }
        Err(e) => ToolResult::err_json(format!("获取 {symbol} 行情失败：{e}")),
    }
}

/// Resolve a symbol + optional market argument into (symbol, market, provider).
fn resolve_symbol_market(
    ctx: &ToolCtx<'_>,
    args: &Value,
) -> Result<(String, String, String), ToolResult> {
    let symbol = match args.get("symbol").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => return Err(ToolResult::err_json("缺少参数 symbol")),
    };
    if symbol.is_empty() {
        return Err(ToolResult::err_json("symbol 不能为空"));
    }
    let market = args
        .get("market")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_uppercase())
        .unwrap_or_else(|| infer_market(&symbol).to_string());
    let config = match quote_provider_service::get_quote_provider_config(ctx.db) {
        Ok(c) => c,
        Err(e) => return Err(ToolResult::err_json(format!("读取行情源配置失败：{e}"))),
    };
    let provider = match market.as_str() {
        "HK" => config.hk_provider,
        "CN" => config.cn_provider,
        _ => config.us_provider,
    };
    Ok((symbol, market, provider))
}

async fn tool_stock_fundamentals(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (symbol, market, provider) = match resolve_symbol_market(ctx, args) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let quote = match market.as_str() {
        "HK" => {
            quote_service::fetch_hk_quote_with_provider(ctx.quote_state, &symbol, &provider).await
        }
        "CN" => {
            quote_service::fetch_cn_quote_with_provider(ctx.quote_state, &symbol, &provider).await
        }
        _ => quote_service::fetch_us_quote_with_provider(ctx.quote_state, &symbol, &provider).await,
    };
    match quote {
        Ok(result) => {
            let q = result.data;
            ToolResult::ok_json(json!({
                "symbol": q.symbol,
                "name": q.name,
                "market": q.market,
                "current_price": q.current_price,
                "pe_ttm": q.pe_ttm,
                "pb": q.pb,
                "market_cap": q.market_cap,
                "dividend_yield": q.dividend_yield,
                "eps": q.eps,
                "roe": q.roe,
                "turnover_rate": q.turnover_rate,
            }))
        }
        Err(e) => ToolResult::err_json(format!("获取 {symbol} 基本面失败：{e}")),
    }
}

async fn tool_technical_indicators(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (symbol, market, provider) = match resolve_symbol_market(ctx, args) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let days = args
        .get("days")
        .and_then(|v| v.as_i64())
        .unwrap_or(120)
        .clamp(30, 365) as usize;
    let end = Utc::now().date_naive();
    let start = end - Duration::days((days as i64) * 2);
    let candles = match quote_service::fetch_stock_candles(
        ctx.quote_state,
        &symbol,
        &market,
        start,
        end,
        &provider,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return ToolResult::err_json(format!("获取 {symbol} K线失败：{e}")),
    };
    if candles.len() < 20 {
        return ToolResult::ok_json(json!({
            "symbol": symbol,
            "note": format!("可用交易日仅 {} 个，不足以计算技术指标", candles.len()),
        }));
    }
    // Latest values of each indicator (the model wants the current reading).
    let last = candles.len() - 1;
    let ma5 = indicators::sma(&candles, 5)[last];
    let ma10 = indicators::sma(&candles, 10)[last];
    let ma20 = indicators::sma(&candles, 20)[last];
    let ma60 = if candles.len() >= 60 {
        indicators::sma(&candles, 60)[last]
    } else {
        None
    };
    let macd = indicators::macd(&candles, 12, 26, 9);
    let macd_last = macd[last];
    let rsi = indicators::rsi(&candles, 14)[last];
    let boll = indicators::bollinger(&candles, 20, 2.0);
    let boll_last = boll[last];
    ToolResult::ok_json(json!({
        "symbol": symbol,
        "market": market,
        "data_points": candles.len(),
        "latest_close": candles[last].close,
        "latest_date": candles[last].date,
        "ma5": ma5,
        "ma10": ma10,
        "ma20": ma20,
        "ma60": ma60,
        "macd_dif": macd_last.dif,
        "macd_dea": macd_last.dea,
        "macd_histogram": macd_last.histogram,
        "rsi14": rsi,
        "bollinger_middle": boll_last.middle,
        "bollinger_upper": boll_last.upper,
        "bollinger_lower": boll_last.lower,
    }))
}

async fn tool_financial_statements(_ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let symbol = match args.get("symbol").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => return ToolResult::err_json("缺少参数 symbol"),
    };
    if symbol.is_empty() {
        return ToolResult::err_json("symbol 不能为空");
    }
    let periods = args
        .get("periods")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .clamp(1, 8) as usize;
    let market = infer_market(&symbol).to_string();
    match quote_service::fetch_financial_statements(&symbol, &market, periods).await {
        Ok(reports) if !reports.is_empty() => {
            ToolResult::ok_json(json!({ "symbol": symbol, "market": market, "periods": reports }))
        }
        Ok(_) => ToolResult::ok_json(
            json!({ "symbol": symbol, "market": market, "note": "未查到财务报表数据" }),
        ),
        Err(e) => ToolResult::err_json(format!("获取 {symbol} 财务报表失败：{e}")),
    }
}

async fn tool_price_history(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let symbol = match args.get("symbol").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => return ToolResult::err_json("缺少参数 symbol"),
    };
    if symbol.is_empty() {
        return ToolResult::err_json("symbol 不能为空");
    }
    let market = args
        .get("market")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_uppercase())
        .unwrap_or_else(|| infer_market(&symbol).to_string());

    let days = args
        .get("days")
        .and_then(|v| v.as_i64())
        .unwrap_or(30)
        .clamp(1, 365) as usize;
    let config = match quote_provider_service::get_quote_provider_config(ctx.db) {
        Ok(c) => c,
        Err(e) => return ToolResult::err_json(format!("读取行情源配置失败：{e}")),
    };
    let provider = match market.as_str() {
        "HK" => &config.hk_provider,
        "CN" => &config.cn_provider,
        _ => &config.us_provider,
    };
    let end = Utc::now().date_naive();
    let start = end - Duration::days(days as i64 * 2); // x2 to clear weekends/holidays
    match quote_service::fetch_stock_history(
        ctx.quote_state,
        &symbol,
        &market,
        start,
        end,
        provider,
    )
    .await
    {
        Ok(history) => {
            // Trim to the last `days` points so we don't ship a year of rows
            // when the model only asked for a week.
            let trimmed: Vec<_> = if history.len() > days {
                history[history.len() - days..].to_vec()
            } else {
                history
            };
            let series: Vec<Value> = trimmed
                .iter()
                .map(|(d, p)| json!({ "date": d.format("%Y-%m-%d").to_string(), "close": ((p * 100.0).round() / 100.0) }))
                .collect();
            ToolResult::ok_json(json!({
                "symbol": symbol,
                "market": market,
                "points": series,
            }))
        }
        Err(e) => ToolResult::err_json(format!("获取 {symbol} 历史价格失败：{e}")),
    }
}

async fn tool_portfolio_overview(ctx: &ToolCtx<'_>) -> ToolResult {
    match build_portfolio_context(ctx.db, ctx.cache, ctx.quote_cache).await {
        Ok(markdown) => ToolResult::ok_json(json!({ "portfolio": markdown })),
        Err(e) => ToolResult::err_json(format!("获取组合总览失败：{e}")),
    }
}

async fn tool_holdings_detail(ctx: &ToolCtx<'_>) -> ToolResult {
    // cache_only = true: tools should not trigger cascading network fetches.
    // The model can call get_stock_quote explicitly for fresh prices.
    match PortfolioReadModel::load(ctx.db, ctx.quote_cache, None, QuoteReadMode::CacheOnly).await {
        Ok(model) => ToolResult::ok_json(json!({ "holdings": model.holdings() })),
        Err(e) => ToolResult::err_json(format!("获取持仓明细失败：{e}")),
    }
}

async fn tool_performance_metrics(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let days = args
        .get("periodDays")
        .and_then(|v| v.as_i64())
        .unwrap_or(365)
        .clamp(1, 3650);
    let end = Utc::now().date_naive();
    let start = end - Duration::days(days);
    let filter = PerformanceFilter::default();
    match performance_service::get_performance_summary(ctx.db, start, end, &filter) {
        Ok(summary) => {
            // The return_series can be large; keep only a compact view so we
            // don't blow the context budget. The headline metrics are what the
            // model actually needs for most questions.
            let compact = json!({
                "start_date": summary.start_date,
                "end_date": summary.end_date,
                "start_value": summary.start_value,
                "end_value": summary.end_value,
                "total_return": summary.total_return,
                "annualized_return": summary.annualized_return,
                "total_pnl": summary.total_pnl,
                "max_drawdown": summary.max_drawdown,
                "volatility": summary.volatility,
                "sharpe_ratio": summary.sharpe_ratio,
                "data_points": summary.return_series.len(),
            });
            ToolResult::ok_json(compact)
        }
        Err(e) => ToolResult::err_json(format!("获取绩效指标失败：{e}")),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// New tools (batch 2)
// ─────────────────────────────────────────────────────────────────────────────

async fn tool_search_stock(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => return ToolResult::err_json("缺少参数 query"),
    };
    if query.is_empty() {
        return ToolResult::err_json("query 不能为空");
    }
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("name_to_symbol");
    match direction {
        "symbol_to_name" => {
            let result =
                lookup_stock_name_by_symbol_with_state(ctx.quote_state, query.clone()).await;
            match result {
                Ok(Some(name)) => ToolResult::ok_json(json!({ "symbol": query, "name": name })),
                Ok(None) => ToolResult::ok_json(
                    json!({ "symbol": query, "name": null, "note": "未找到对应名称" }),
                ),
                Err(e) => ToolResult::err_json(format!("查询名称失败：{e}")),
            }
        }
        _ => {
            let result = lookup_cn_stock_code_with_state(ctx.quote_state, query.clone()).await;
            match result {
                Ok(Some(code)) => {
                    // lookup returns lowercased code (e.g. "sh600519"); normalise to the
                    // uppercase form the rest of the app expects (SH600519).
                    let normalized = code.to_uppercase();
                    ToolResult::ok_json(
                        json!({ "name": query, "symbol": normalized, "market": "CN" }),
                    )
                }
                Ok(None) => ToolResult::ok_json(
                    json!({ "name": query, "symbol": null, "note": "未找到对应 A 股代码；该名称可能为港股或美股，请尝试直接使用代码" }),
                ),
                Err(e) => ToolResult::err_json(format!("查询代码失败：{e}")),
            }
        }
    }
}

async fn tool_transactions(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let tx_type = args.get("txType").and_then(|v| v.as_str());
    let symbol = args.get("symbol").and_then(|v| v.as_str());
    let days = args.get("days").and_then(|v| v.as_i64());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .map(|l| l as usize);
    match query_transactions_inner(ctx.db, None, symbol, tx_type, days, limit) {
        Ok(txns) => ToolResult::ok_json(json!({ "transactions": txns, "count": txns.len() })),
        Err(e) => ToolResult::err_json(format!("查询交易记录失败：{e}")),
    }
}

/// Compute the (start, end) NaiveDate window from a `periodDays` arg, clamped
/// to a sane maximum. Shared by all the performance-family tools.
fn period_window(args: &Value) -> (chrono::NaiveDate, chrono::NaiveDate) {
    let days = args
        .get("periodDays")
        .and_then(|v| v.as_i64())
        .unwrap_or(365)
        .clamp(1, 3650);
    let end = Utc::now().date_naive();
    (end - Duration::days(days), end)
}

async fn tool_return_attribution(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (start, end) = period_window(args);
    let filter = PerformanceFilter::default();
    match performance_service::get_return_attribution(ctx.db, start, end, &filter) {
        Ok(attr) => {
            // by_holding can be long; cap at top 15 by absolute contribution.
            let mut holdings = attr.by_holding.clone();
            holdings.sort_by(|a, b| {
                b.contribution_percent
                    .abs()
                    .partial_cmp(&a.contribution_percent.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            holdings.truncate(15);
            ToolResult::ok_json(json!({
                "total_pnl": attr.total_pnl,
                "by_market": attr.by_market,
                "by_category": attr.by_category,
                "by_holding_top15": holdings,
            }))
        }
        Err(e) => ToolResult::err_json(format!("收益归因失败：{e}")),
    }
}

async fn tool_monthly_returns(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (start, end) = period_window(args);
    let filter = PerformanceFilter::default();
    match performance_service::get_monthly_returns(ctx.db, start, end, &filter) {
        Ok(returns) => ToolResult::ok_json(json!({ "monthly_returns": returns })),
        Err(e) => ToolResult::err_json(format!("月度收益查询失败：{e}")),
    }
}

async fn tool_drawdown_analysis(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (start, end) = period_window(args);
    let filter = PerformanceFilter::default();
    match performance_service::get_drawdown_analysis(ctx.db, start, end, &filter) {
        Ok(dd) => ToolResult::ok_json(json!({
            "max_drawdown": dd.max_drawdown,
            "peak_date": dd.peak_date,
            "trough_date": dd.trough_date,
            "recovery_date": dd.recovery_date,
            "drawdown_duration_days": dd.drawdown_duration,
            "recovery_duration_days": dd.recovery_duration,
        })),
        Err(e) => ToolResult::err_json(format!("回撤分析失败：{e}")),
    }
}

async fn tool_risk_metrics(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (start, end) = period_window(args);
    let filter = PerformanceFilter::default();
    match performance_service::get_risk_metrics(ctx.db, start, end, &filter) {
        Ok(m) => ToolResult::ok_json(json!({
            "daily_volatility": m.daily_volatility,
            "annualized_volatility": m.annualized_volatility,
            "sharpe_ratio": m.sharpe_ratio,
            "max_drawdown": m.max_drawdown,
            "calmar_ratio": m.calmar_ratio,
            "risk_free_rate": m.risk_free_rate,
        })),
        Err(e) => ToolResult::err_json(format!("风险指标查询失败：{e}")),
    }
}

async fn tool_holding_ranking(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let (start, end) = period_window(args);
    let sort_by = args.get("sortBy").and_then(|v| v.as_str()).unwrap_or("pnl");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    let filter = PerformanceFilter::default();
    match performance_service::get_holding_performance_ranking(
        ctx.db, start, end, sort_by, limit, &filter,
    ) {
        Ok(ranking) => ToolResult::ok_json(json!({ "ranking": ranking, "sort_by": sort_by })),
        Err(e) => ToolResult::err_json(format!("持仓排名查询失败：{e}")),
    }
}

fn dashboard_rates_or_error(
    result: Result<ExchangeRates, String>,
) -> Result<ExchangeRates, ToolResult> {
    result.map_err(|error| ToolResult::err_json(format!("仪表盘总览失败：{error}")))
}

async fn tool_dashboard_summary(ctx: &ToolCtx<'_>) -> ToolResult {
    // Mirror get_dashboard_summary but with cache_only=true so tools never
    // trigger cascading network fetches (the model can call get_stock_quote
    // explicitly if it needs a fresh price).
    let rates = match dashboard_rates_or_error(get_cached_rates(ctx.cache, ctx.db).await) {
        Ok(rates) => rates,
        Err(error) => return error,
    };
    let base = "USD";
    match PortfolioReadModel::load(ctx.db, ctx.quote_cache, None, QuoteReadMode::CacheOnly).await {
        Ok(model) => {
            let mut us_mv = 0.0f64;
            let mut cn_mv = 0.0f64;
            let mut hk_mv = 0.0f64;
            let mut total_cost = 0.0f64;
            let mut daily_pnl = 0.0f64;
            for d in model.holdings() {
                let mv = convert_currency(d.market_value, &d.currency, base, &rates);
                let cv = convert_currency(d.cost_value, &d.currency, base, &rates);
                daily_pnl += convert_currency(d.daily_pnl, &d.currency, base, &rates);
                match d.market.as_str() {
                    "US" => us_mv += mv,
                    "CN" => cn_mv += mv,
                    "HK" => hk_mv += mv,
                    _ => {}
                }
                total_cost += cv;
            }
            let total_mv = us_mv + cn_mv + hk_mv;
            let total_pnl = total_mv - total_cost;
            let total_pnl_pct = if total_cost != 0.0 {
                total_pnl / total_cost * 100.0
            } else {
                0.0
            };
            let summary = DashboardSummary {
                total_market_value: total_mv,
                total_cost,
                total_pnl,
                total_pnl_percent: total_pnl_pct,
                daily_pnl,
                us_market_value: us_mv,
                cn_market_value: cn_mv,
                hk_market_value: hk_mv,
                exchange_rates: rates,
                base_currency: base.to_string(),
            };
            ToolResult::ok_json(json!(summary))
        }
        Err(e) => ToolResult::err_json(format!("仪表盘总览失败：{e}")),
    }
}

async fn tool_dividend_income(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    // Aggregate PAY-type transactions (dividends/interest) by symbol.
    // Net income per row = total_amount - commission.
    let days = args.get("days").and_then(|v| v.as_i64());
    match query_transactions_inner(ctx.db, None, None, Some("PAY"), days, None) {
        Ok(txns) => {
            let mut by_symbol: std::collections::HashMap<String, (String, String, f64, i64)> =
                std::collections::HashMap::new();
            let mut grand_total = 0.0f64;
            let mut count = 0i64;
            for t in &txns {
                let net = t.total_amount - t.commission;
                grand_total += net;
                count += 1;
                let entry = by_symbol
                    .entry(t.symbol.clone())
                    .or_insert_with(|| (t.name.clone(), t.currency.clone(), 0.0, 0));
                entry.2 += net;
                entry.3 += 1;
            }
            let mut rows: Vec<Value> = by_symbol
                .into_iter()
                .map(|(symbol, (name, currency, total, n))| {
                    json!({
                        "symbol": symbol,
                        "name": name,
                        "currency": currency,
                        "net_income": (total * 100.0).round() / 100.0,
                        "count": n,
                    })
                })
                .collect();
            rows.sort_by(|a, b| {
                b["net_income"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .partial_cmp(&a["net_income"].as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            ToolResult::ok_json(json!({
                "total_net_income": (grand_total * 100.0).round() / 100.0,
                "payment_count": count,
                "by_symbol": rows,
            }))
        }
        Err(e) => ToolResult::err_json(format!("分红查询失败：{e}")),
    }
}

async fn tool_check_alerts(ctx: &ToolCtx<'_>) -> ToolResult {
    // First list the user's configured alerts; then, if we have cached quotes,
    // check which have triggered. check_alerts needs a quote map keyed by
    // symbol → (price, change_pct, pnl_pct); we build it from the cache.
    let alerts = match alert_service::get_alerts(ctx.db) {
        Ok(a) => a,
        Err(e) => return ToolResult::err_json(format!("读取价格提醒失败：{e}")),
    };
    let mut quote_map: std::collections::HashMap<String, (f64, f64, f64)> =
        std::collections::HashMap::new();
    for a in &alerts {
        if quote_map.contains_key(&a.symbol) {
            continue;
        }
        if let Some(q) = ctx.quote_cache.get(&a.symbol) {
            quote_map.insert(a.symbol.clone(), (q.current_price, q.change_percent, 0.0));
        }
    }
    let triggered = alert_service::check_alerts(ctx.db, &quote_map).unwrap_or_default();
    ToolResult::ok_json(json!({
        "total_alerts": alerts.len(),
        "alerts": alerts,
        "triggered": triggered,
        "triggered_count": triggered.len(),
    }))
}

async fn tool_option_positions(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let account_id = match args.get("accountId").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => return ToolResult::err_json("缺少参数 accountId"),
    };
    if account_id.is_empty() {
        return ToolResult::err_json("accountId 不能为空");
    }
    match get_option_contracts_inner(ctx.db, &account_id) {
        Ok(contracts) => {
            let active: Vec<_> = contracts.iter().filter(|c| c.status == "active").collect();
            ToolResult::ok_json(json!({
                "contracts": contracts,
                "total": contracts.len(),
                "active_count": active.len(),
            }))
        }
        Err(e) => ToolResult::err_json(format!("期权持仓查询失败：{e}")),
    }
}

fn option_review_payload(
    mut report: OptionReviewReport,
    symbol: Option<&str>,
) -> Result<Value, String> {
    let symbol = symbol.map(str::trim).filter(|symbol| !symbol.is_empty());
    if let Some(symbol) = symbol {
        report
            .underlyings
            .retain(|review| review.underlying.eq_ignore_ascii_case(symbol));
        if report.underlyings.is_empty() {
            return Err(format!("账户中没有 {symbol} 的期权复盘数据"));
        }
        let mut payload = serde_json::to_value(report).map_err(|error| error.to_string())?;
        let object = payload
            .as_object_mut()
            .ok_or_else(|| "期权复盘结果序列化失败".to_string())?;
        object.insert(
            "scope_note".to_string(),
            json!("summary为账户级；underlyings已按个股过滤"),
        );
        return Ok(payload);
    }
    serde_json::to_value(report).map_err(|error| error.to_string())
}

fn option_review_period_days(args: &Value) -> Option<i64> {
    if args
        .get("allHistory")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    Some(
        args.get("periodDays")
            .and_then(Value::as_i64)
            .unwrap_or(365)
            .clamp(1, 3650),
    )
}

async fn tool_option_review(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let account_id = match args.get("accountId").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return ToolResult::err_json("缺少参数 accountId"),
    };
    let period_days = option_review_period_days(args);
    let report = match option_review_service::get_option_review(ctx.db, account_id, period_days) {
        Ok(report) => report,
        Err(error) => return ToolResult::err_json(format!("期权复盘失败：{error}")),
    };
    let symbol = args.get("symbol").and_then(Value::as_str);
    match option_review_payload(report, symbol) {
        Ok(payload) => ToolResult::ok_json(payload),
        Err(error) => ToolResult::err_json(error),
    }
}

fn required_trimmed_string(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(Value::String(_)) => Err(format!("参数 {key} 不能为空。")),
        Some(_) => Err(format!("参数 {key} 必须是字符串。")),
        None => Err(format!("缺少参数 {key}。")),
    }
}

fn require_allowed_object<'a>(
    value: &'a Value,
    label: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("参数 {label} 必须是 JSON 对象。"))?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("参数 {label} 包含未知字段 {key}。"));
    }
    Ok(object)
}

fn optional_trimmed_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_string()))
        }
        Some(Value::String(_)) => Err(format!("参数 {key} 不能为空。")),
        Some(_) => Err(format!("参数 {key} 必须是字符串。")),
    }
}

fn parse_stock_review_query(args: &Value) -> Result<StockOperationReviewQuery, String> {
    require_allowed_object(
        args,
        "get_stock_review",
        &[
            "start_date",
            "end_date",
            "base_currency",
            "account_id",
            "market",
            "symbol",
        ],
    )?;
    let start = required_trimmed_string(args, "start_date")?;
    let end = required_trimmed_string(args, "end_date")?;
    let start_date = NaiveDate::parse_from_str(&start, "%Y-%m-%d")
        .map_err(|_| "参数 start_date 格式无效，请使用 YYYY-MM-DD。".to_string())?;
    let end_date = NaiveDate::parse_from_str(&end, "%Y-%m-%d")
        .map_err(|_| "参数 end_date 格式无效，请使用 YYYY-MM-DD。".to_string())?;
    let query = StockOperationReviewQuery {
        start_date,
        end_date,
        account_id: optional_trimmed_string(args, "account_id")?,
        market: optional_trimmed_string(args, "market")?.map(|value| value.to_ascii_uppercase()),
        base_currency: required_trimmed_string(args, "base_currency")?.to_ascii_uppercase(),
    };
    stock_operation_review_service::validate_query(&query)?;
    Ok(query)
}

async fn tool_stock_review(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let query = match parse_stock_review_query(args) {
        Ok(query) => query,
        Err(error) => return ToolResult::err_json(error),
    };
    let symbol = match optional_trimmed_string(args, "symbol") {
        Ok(symbol) => symbol.and_then(|symbol| normalize_stock_symbol(&symbol)),
        Err(error) => return ToolResult::err_json(error),
    };
    let report = match stock_operation_review_service::get_stock_operation_review_with_refresh(
        ctx.db,
        Some(ctx.quote_state),
        query,
        true,
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            return ToolResult::err_json(format!("股票操作复盘参数或数据准备失败：{error}"))
        }
    };
    let report = symbol
        .as_deref()
        .map(|symbol| {
            stock_operation_review_service::scope_report_to_symbol(report.clone(), symbol)
        })
        .unwrap_or(report);
    ToolResult::ok_json(json!({
        "deterministic_source": "stock_operation_review_service",
        "scope": { "symbol": symbol },
        "report": report,
        "assistant_policy": {
            "endpoint_effect_is_hindsight_price_comparison": true,
            "not_portfolio_twr_attribution": true,
            "unallocated_dividends_excluded": true,
            "do_not_label_decision_right_or_wrong_from_price_alone": true,
            "maximum_follow_up_questions": 3
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_tool_turns_unavailable_rates_into_explicit_error_data() {
        let error =
            dashboard_rates_or_error(Err("no verified exchange rates".to_string())).unwrap_err();

        assert!(!error.ok);
        let payload: Value = serde_json::from_str(&error.content).unwrap();
        assert_eq!(
            payload["error"],
            "仪表盘总览失败：no verified exchange rates"
        );
    }

    fn option_review_fixture() -> crate::models::option_review::OptionReviewReport {
        use crate::models::option_review::{
            OptionReviewDataQuality, OptionReviewReport, OptionReviewSummary,
            OptionUnderlyingReview,
        };

        OptionReviewReport {
            account_id: "account-1".to_string(),
            currency: "USD".to_string(),
            period_days: Some(365),
            generated_at: "2026-08-24".to_string(),
            summary: OptionReviewSummary {
                completed_campaigns: 2,
                active_campaigns: 0,
                gross_premium: 300.0,
                net_premium_pnl: 240.0,
                completed_gross_premium: 300.0,
                completed_net_premium_pnl: 240.0,
                retention_rate: Some(0.8),
                annualized_yield_on_notional: Some(0.12),
                worst_campaign: None,
            },
            underlyings: vec![
                OptionUnderlyingReview {
                    underlying: "AAPL".to_string(),
                    completed_campaigns: 1,
                    active_campaigns: 0,
                    gross_premium: 200.0,
                    net_premium_pnl: 180.0,
                    completed_gross_premium: 200.0,
                    completed_net_premium_pnl: 180.0,
                    retention_rate: Some(0.9),
                    annualized_yield_on_notional: Some(0.15),
                    worst_campaign_pnl: Some(180.0),
                    flags: Vec::new(),
                    campaigns: Vec::new(),
                },
                OptionUnderlyingReview {
                    underlying: "MSFT".to_string(),
                    completed_campaigns: 1,
                    active_campaigns: 0,
                    gross_premium: 100.0,
                    net_premium_pnl: 60.0,
                    completed_gross_premium: 100.0,
                    completed_net_premium_pnl: 60.0,
                    retention_rate: Some(0.6),
                    annualized_yield_on_notional: Some(0.09),
                    worst_campaign_pnl: Some(60.0),
                    flags: Vec::new(),
                    campaigns: Vec::new(),
                },
            ],
            data_quality: OptionReviewDataQuality {
                excluded_open_campaigns: 0,
                unmatched_records: 0,
                missing_trade_dates: 0,
                notes: Vec::new(),
            },
        }
    }

    #[test]
    fn tool_definitions_are_valid_json() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 22);
        for d in &defs {
            assert_eq!(d["type"], "function");
            assert!(d["function"]["name"].is_string());
            assert!(d["function"]["description"].is_string());
            assert!(d["function"]["parameters"]["type"] == "object");
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let defs = tool_definitions();
        let names: std::collections::HashSet<&str> = defs
            .iter()
            .map(|d| d["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 22);
    }

    #[test]
    fn stock_review_exposes_one_read_only_tool() {
        let defs = tool_definitions();
        let names: Vec<_> = defs
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"get_stock_review"));
        assert!(!names.contains(&"save_stock_review_annotation"));

        let read = defs
            .iter()
            .find(|tool| tool["function"]["name"] == "get_stock_review")
            .expect("get_stock_review definition");
        assert_eq!(
            read["function"]["parameters"]["required"],
            json!(["start_date", "end_date", "base_currency"])
        );
        assert_eq!(
            read["function"]["parameters"]["additionalProperties"],
            false
        );
        let properties = read["function"]["parameters"]["properties"]
            .as_object()
            .unwrap();
        assert_eq!(
            properties
                .keys()
                .map(String::as_str)
                .collect::<std::collections::HashSet<_>>(),
            [
                "start_date",
                "end_date",
                "base_currency",
                "account_id",
                "market",
                "symbol",
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn lightweight_stock_review_parser_rejects_legacy_benchmark_and_campaign_arguments() {
        let benchmark = parse_stock_review_query(&json!({
            "start_date": "2026-07-01",
            "end_date": "2026-08-30",
            "base_currency": "CNY",
            "benchmark_symbol": "000300.SS"
        }))
        .unwrap_err();
        assert!(benchmark.contains("benchmark_symbol"));

        let campaign = parse_stock_review_query(&json!({
            "start_date": "2026-07-01",
            "end_date": "2026-08-30",
            "base_currency": "CNY",
            "campaign_id": "campaign-1"
        }))
        .unwrap_err();
        assert!(campaign.contains("campaign_id"));
    }

    #[tokio::test]
    async fn stock_review_dispatch_uses_deterministic_service_and_unavailable_is_success_data() {
        let db = Database::new(":memory:").unwrap();
        let cache = ExchangeRateCache::new();
        let quote_cache = QuoteCache::new();
        let quote_state = QuoteServiceState::new();
        let ctx =
            ToolCtx::for_untrusted_model_turn(&db, &cache, &quote_cache, &quote_state, "untrusted");
        let result = execute_tool(
            &ctx,
            "get_stock_review",
            r#"{"start_date":"2026-01-01","end_date":"2026-01-31","base_currency":"USD"}"#,
        )
        .await;
        assert!(result.ok, "{}", result.content);
        let payload: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            payload["deterministic_source"],
            "stock_operation_review_service"
        );
        assert!(payload["report"]["summary"].is_object());
        assert!(payload["report"]["data_quality"].is_object());
        assert!(payload.get("campaign_detail").is_none());
    }

    #[tokio::test]
    async fn stock_review_read_tool_rejects_unknown_and_invalid_structured_fields() {
        let db = Database::new(":memory:").unwrap();
        let cache = ExchangeRateCache::new();
        let quote_cache = QuoteCache::new();
        let quote_state = QuoteServiceState::new();
        let ctx =
            ToolCtx::for_untrusted_model_turn(&db, &cache, &quote_cache, &quote_state, "untrusted");

        for (arguments, field) in [
            (
                r#"{"start_date":"2026-01-01","end_date":"2026-01-31","base_currency":"USD","account_id":7}"#,
                "account_id",
            ),
            (
                r#"{"start_date":"2026-01-01","end_date":"2026-01-31","base_currency":"USD","symbol":" "}"#,
                "symbol",
            ),
            (
                r#"{"start_date":"2026-01-01","end_date":"2026-01-31","base_currency":"USD","campaign_id":null}"#,
                "campaign_id",
            ),
            (
                r#"{"start_date":"2026-01-01","end_date":"2026-01-31","base_currency":"USD","unexpected":true}"#,
                "unexpected",
            ),
        ] {
            let bad_read = execute_tool(&ctx, "get_stock_review", arguments).await;
            assert!(!bad_read.ok, "invalid read arguments accepted: {arguments}");
            assert!(bad_read.content.contains(field), "{}", bad_read.content);
        }
    }

    #[test]
    fn stock_review_parameter_errors_are_actionable() {
        let error = parse_stock_review_query(&json!({
            "start_date": "2026/01/01",
            "end_date": "2026-01-31",
            "base_currency": "USD"
        }))
        .unwrap_err();
        assert!(error.contains("start_date"));
        assert!(error.contains("YYYY-MM-DD"));
    }

    #[test]
    fn option_review_tool_requires_account_and_supports_filters() {
        let defs = tool_definitions();
        let tool = defs
            .iter()
            .find(|d| d["function"]["name"] == "get_option_review")
            .expect("get_option_review definition");
        assert_eq!(
            tool["function"]["parameters"]["required"],
            json!(["accountId"])
        );
        assert_eq!(
            tool["function"]["parameters"]["properties"]["periodDays"]["maximum"],
            3650
        );
        assert!(tool["function"]["parameters"]["properties"]["symbol"].is_object());
        assert_eq!(
            tool["function"]["parameters"]["properties"]["allHistory"]["type"],
            "boolean"
        );
        assert_eq!(
            tool["function"]["parameters"]["properties"]["allHistory"]["default"],
            false
        );
    }

    #[test]
    fn option_review_period_arguments_all_history_overrides_period_days() {
        assert_eq!(
            option_review_period_days(&json!({ "allHistory": true, "periodDays": 30 })),
            None
        );
    }

    #[test]
    fn option_review_period_arguments_default_to_recent_year() {
        assert_eq!(option_review_period_days(&json!({})), Some(365));
        assert_eq!(
            option_review_period_days(&json!({ "allHistory": false })),
            Some(365)
        );
    }

    #[test]
    fn option_review_period_arguments_clamp_recent_range() {
        assert_eq!(
            option_review_period_days(&json!({ "periodDays": 0 })),
            Some(1)
        );
        assert_eq!(
            option_review_period_days(&json!({ "periodDays": 9999 })),
            Some(3650)
        );
    }

    #[test]
    fn option_review_symbol_filter_is_case_insensitive_and_preserves_account_summary() {
        let payload = option_review_payload(option_review_fixture(), Some("aapl")).unwrap();
        assert_eq!(payload["summary"]["completed_campaigns"], 2);
        assert_eq!(payload["underlyings"].as_array().unwrap().len(), 1);
        assert_eq!(payload["underlyings"][0]["underlying"], "AAPL");
        assert!(payload["scope_note"].as_str().unwrap().contains("账户级"));
    }

    #[test]
    fn option_review_symbol_filter_reports_missing_symbol() {
        let error = option_review_payload(option_review_fixture(), Some("NVDA")).unwrap_err();
        assert!(error.contains("NVDA"));
    }

    #[test]
    fn infer_market_handles_common_formats() {
        assert_eq!(infer_market("AAPL"), "US");
        assert_eq!(infer_market("0700.HK"), "HK");
        assert_eq!(infer_market("9988.HK"), "HK");
        assert_eq!(infer_market("SH600519"), "CN");
        assert_eq!(infer_market("SZ000001"), "CN");
        assert_eq!(infer_market("600519"), "CN");
        assert_eq!(infer_market("00700"), "HK");
    }

    // execute_tool is async and needs a live AppHandle/Database, so we cover the
    // pure pieces here (schema + inference) and rely on the chat-loop integration
    // for end-to-end coverage. The unknown-tool branch is trivially correct.
}
