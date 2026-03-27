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
  { value: "eastmoney", label: "东方财富（默认）" },
  { value: "xueqiu", label: "雪球" },
];

const PROVIDER_OPTIONS_CN = [
  { value: "eastmoney", label: "东方财富（默认）" },
  { value: "xueqiu", label: "雪球" },
];

export default function GeneralSettings() {
  const { refreshIntervalMs, setRefreshInterval } = useQuoteStore();
  const [providerConfig, setProviderConfig] = useState<QuoteProviderConfig>({
    us_provider: "eastmoney",
    hk_provider: "eastmoney",
    cn_provider: "eastmoney",
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
              extra="从浏览器中复制雪球的 Cookie，粘贴到此处。步骤：登录 xueqiu.com → 按 F12 打开开发者工具 → Application → Cookies → 找到 xq_a_token，复制其值"
            >
              <Input.TextArea
                rows={3}
                placeholder="粘贴 xq_a_token 的值"
                value={providerConfig.xueqiu_cookie ?? ""}
                onChange={(e) =>
                  setProviderConfig({ ...providerConfig, xueqiu_cookie: e.target.value || null })
                }
                onBlur={(e) => handleCookieSave(e.target.value)}
              />
            </Form.Item>
          </Form>
          <Paragraph type="secondary">
            雪球 API 需要登录后的 Cookie 才能访问。如果遇到 400 错误，请在浏览器中登录雪球账号，然后将 Cookie 粘贴到上方输入框中。Cookie 可能会过期，届时需要重新获取。
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
