import { useEffect, useState } from "react";
import { Card, Form, Input, Select, Typography, message } from "antd";
import { invoke } from "@tauri-apps/api/core";
import { useQuoteStore } from "../../stores/quoteStore";
import type { QuoteProviderConfig } from "../../types";

const { Paragraph } = Typography;

const INTERVAL_OPTIONS = [
  { value: 60_000, label: "1 分钟" },
  { value: 2 * 60_000, label: "2 分钟" },
  { value: 5 * 60_000, label: "5 分钟（默认）" },
  { value: 10 * 60_000, label: "10 分钟" },
  { value: 15 * 60_000, label: "15 分钟" },
  { value: 30 * 60_000, label: "30 分钟" },
];

const PROVIDER_OPTIONS_US_HK = [
  { value: "yahoo", label: "Yahoo Finance" },
  { value: "eastmoney", label: "东方财富" },
  { value: "xueqiu", label: "雪球（默认）" },
];

const PROVIDER_OPTIONS_CN = [
  { value: "eastmoney", label: "东方财富" },
  { value: "xueqiu", label: "雪球（默认）" },
];

export default function GeneralSettings() {
  const { refreshIntervalMs, setRefreshInterval } = useQuoteStore();
  const [providerConfig, setProviderConfig] = useState<QuoteProviderConfig>({
    us_provider: "xueqiu",
    hk_provider: "xueqiu",
    cn_provider: "xueqiu",
    xueqiu_cookie: null,
  });

  useEffect(() => {
    invoke<QuoteProviderConfig>("get_quote_provider_config")
      .then(setProviderConfig)
      .catch(() => {
        // Use defaults on error
      });
  }, []);

  const handleIntervalChange = (value: number) => {
    setRefreshInterval(value);
    message.success("刷新频率已更新");
  };

  const handleProviderChange = async (
    market: keyof QuoteProviderConfig,
    value: string
  ) => {
    const updated = { ...providerConfig, [market]: value };
    try {
      await invoke("update_quote_provider_config", { config: updated });
      setProviderConfig(updated);
      message.success("行情数据源已更新");
    } catch (err) {
      message.error("更新失败: " + String(err));
    }
  };

  const handleCookieSave = async (cookieValue: string) => {
    const updated = { ...providerConfig, xueqiu_cookie: cookieValue || null };
    try {
      await invoke("update_quote_provider_config", { config: updated });
      setProviderConfig(updated);
      message.success("雪球 Cookie 已更新");
    } catch (err) {
      message.error("更新失败: " + String(err));
    }
  };

  const isXueqiuUsed =
    providerConfig.us_provider === "xueqiu" ||
    providerConfig.hk_provider === "xueqiu" ||
    providerConfig.cn_provider === "xueqiu";

  return (
    <div className="space-y-6">
      <Card title="行情数据源设置">
        <Form layout="vertical" style={{ maxWidth: 400 }}>
          <Form.Item label="美股数据源">
            <Select
              value={providerConfig.us_provider}
              onChange={(v) => handleProviderChange("us_provider", v)}
              options={PROVIDER_OPTIONS_US_HK}
            />
          </Form.Item>
          <Form.Item label="港股数据源">
            <Select
              value={providerConfig.hk_provider}
              onChange={(v) => handleProviderChange("hk_provider", v)}
              options={PROVIDER_OPTIONS_US_HK}
            />
          </Form.Item>
          <Form.Item label="A股数据源">
            <Select
              value={providerConfig.cn_provider}
              onChange={(v) => handleProviderChange("cn_provider", v)}
              options={PROVIDER_OPTIONS_CN}
            />
          </Form.Item>
        </Form>
        <Paragraph type="secondary">
          各市场的行情数据来源：A股支持东方财富和雪球，港股和美股支持 Yahoo Finance、东方财富和雪球。修改后将在下次刷新时生效。
        </Paragraph>
      </Card>

      {isXueqiuUsed && (
        <Card title="雪球 Cookie 设置">
          <Form layout="vertical" style={{ maxWidth: 600 }}>
            <Form.Item
              label="雪球 Cookie"
              extra="从浏览器中复制雪球的完整 Cookie 字符串。步骤：登录 xueqiu.com → 按 F12 → Network → 刷新页面 → 点击任意请求 → 复制 Request Headers 中的 Cookie 值（完整字符串，包含 xq_a_token、xq_id_token 等多个字段）"
            >
              <Input.TextArea
                rows={3}
                placeholder="粘贴完整 Cookie 字符串，例如：xq_a_token=xxx; xq_id_token=xxx; xq_r_token=xxx; xqat=xxx; u=xxx"
                value={providerConfig.xueqiu_cookie ?? ""}
                onChange={(e) =>
                  setProviderConfig({ ...providerConfig, xueqiu_cookie: e.target.value || null })
                }
                onBlur={(e) => handleCookieSave(e.target.value)}
              />
            </Form.Item>
          </Form>
          <Paragraph type="secondary">
            雪球历史行情 API 需要完整的登录 Cookie（包括 xq_a_token、xq_id_token、xq_r_token、xqat 等）。仅提供 xq_a_token 可能不足以获取历史K线数据。Cookie 可能会过期，届时需要重新获取。
          </Paragraph>
        </Card>
      )}

      <Card title="行情刷新设置">
        <Form layout="vertical" style={{ maxWidth: 400 }}>
          <Form.Item label="自动刷新频率">
            <Select
              value={refreshIntervalMs}
              onChange={handleIntervalChange}
              options={INTERVAL_OPTIONS}
            />
          </Form.Item>
        </Form>
        <Paragraph type="secondary">
          设置持仓行情的自动刷新间隔时间，应用到所有行情数据的自动刷新。修改后将立即生效。
        </Paragraph>
      </Card>
    </div>
  );
}
