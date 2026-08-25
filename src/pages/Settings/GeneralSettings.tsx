import { useCallback, useEffect, useState } from "react";
import { Button, Card, Checkbox, Form, Input, Modal, Radio, Select, Space, Tabs, Typography, message } from "antd";
import { invoke } from "@tauri-apps/api/core";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { useSettingsStore, type ColorScheme, type ThemeMode } from "../../stores/settingsStore";
import { useXueqiuLogin } from "../../hooks/useXueqiuLogin";
import type { QuoteProviderConfig } from "../../types";

const { Paragraph } = Typography;

const PROVIDER_OPTIONS_US_HK = [
  { value: "yahoo", label: "Yahoo Finance" },
  { value: "eastmoney", label: "东方财富" },
  { value: "xueqiu", label: "雪球（默认）" },
];

const PROVIDER_OPTIONS_CN = [
  { value: "eastmoney", label: "东方财富" },
  { value: "xueqiu", label: "雪球（默认）" },
];

const COLOR_SCHEME_OPTIONS: { value: ColorScheme; label: string }[] = [
  { value: "red-up", label: "红涨绿跌（A股风格）" },
  { value: "green-up", label: "绿涨红跌（美股风格）" },
];

const THEME_OPTIONS: { value: ThemeMode; label: string; icon: string }[] = [
  { value: "light", label: "浅色模式", icon: "☀️" },
  { value: "dark", label: "深色模式", icon: "🌙" },
  { value: "system", label: "跟随系统", icon: "💻" },
];

// The default quote provider config, matching the Rust
// `impl Default for QuoteProviderConfig`. Used to reset the local UI state
// after a factory reset so the form doesn't briefly show stale values.
const DEFAULT_PROVIDER_CONFIG: QuoteProviderConfig = {
  us_provider: "xueqiu",
  hk_provider: "xueqiu",
  cn_provider: "xueqiu",
  xueqiu_cookie: null,
  xueqiu_u: null,
  cn_adjust_sell_pay_cost: true,
  us_adjust_sell_pay_cost: false,
  hk_adjust_sell_pay_cost: false,
};

// localStorage keys that hold UI preferences (the only front-end persisted
// state — everything else lives in the Tauri SQLite backend).
const LOCAL_STORAGE_KEYS = [
  "pnl_color_scheme",
  "app_theme_mode",
  "base_currency",
  "statistics_selected_market",
  "holdings_table_page_size",
  "options_selected_account_id",
] as const;

export default function GeneralSettings() {
  const { colorScheme, setColorScheme, themeMode, setThemeMode } = useSettingsStore();
  const { setBaseCurrency } = useExchangeRateStore();
  const [providerConfig, setProviderConfig] = useState<QuoteProviderConfig>({
    us_provider: "xueqiu",
    hk_provider: "xueqiu",
    cn_provider: "xueqiu",
    xueqiu_cookie: null,
    xueqiu_u: null,
    cn_adjust_sell_pay_cost: true,
    us_adjust_sell_pay_cost: false,
    hk_adjust_sell_pay_cost: false,
  });
  const [recalculating, setRecalculating] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [pastingRaw, setPastingRaw] = useState("");
  const [parsing, setParsing] = useState(false);

  // Factory-reset confirmation state. The checkbox starts unchecked every
  // time the modal opens so the user must actively re-acknowledge the risk.
  const [resetModalOpen, setResetModalOpen] = useState(false);
  const [resetAcknowledged, setResetAcknowledged] = useState(false);
  const [resetting, setResetting] = useState(false);

  // Single source of truth for "capture succeeded": both the explicit button
  // and the auto-capture-on-close path flow through here, so the toast fires
  // exactly once per capture regardless of how many component instances or
  // StrictMode remounts exist.
  const handleCaptured = useCallback((config: QuoteProviderConfig) => {
    setProviderConfig(config);
    message.success("已从雪球登录会话抓取 Cookie 并保存");
  }, []);
  const { loginWindowOpen, openLoginWindow, capture } =
    useXueqiuLogin(handleCaptured);

  useEffect(() => {
    invoke<QuoteProviderConfig>("get_quote_provider_config")
      .then(setProviderConfig)
      .catch(() => {
        // Use defaults on error
      });
  }, []);

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

  const handleUValueSave = async (uValue: string) => {
    const updated = { ...providerConfig, xueqiu_u: uValue || null };
    try {
      await invoke("update_quote_provider_config", { config: updated });
      setProviderConfig(updated);
      message.success("雪球用户 ID 已更新");
    } catch (err) {
      message.error("更新失败: " + String(err));
    }
  };

  const handleOpenLoginWindow = async () => {
    try {
      await openLoginWindow();
      message.info("已打开雪球登录窗口，请在新窗口内完成登录后再点击「我已登录，抓取 Cookie」");
    } catch (err) {
      message.error("打开登录窗口失败: " + String(err));
    }
  };

  const handleCapture = async () => {
    setCapturing(true);
    try {
      // capture() fires `handleCaptured` on success, which sets state and
      // shows the toast — so we deliberately don't duplicate that here.
      await capture();
    } catch (err) {
      message.error(String(err));
    } finally {
      setCapturing(false);
    }
  };

  const handleParsePaste = async () => {
    if (!pastingRaw.trim()) {
      message.warning("请先粘贴 Cookie 内容");
      return;
    }
    setParsing(true);
    try {
      const updated = await invoke<QuoteProviderConfig>(
        "parse_xueqiu_cookie_text",
        { raw: pastingRaw, existing: providerConfig }
      );
      setProviderConfig(updated);
      setPastingRaw("");
      message.success("已解析 Cookie 并保存");
    } catch (err) {
      message.error(String(err));
    } finally {
      setParsing(false);
    }
  };

  const handleCostAdjustChange = async (
    key: "cn_adjust_sell_pay_cost" | "us_adjust_sell_pay_cost" | "hk_adjust_sell_pay_cost",
    checked: boolean
  ) => {
    const updated = { ...providerConfig, [key]: checked };
    try {
      await invoke("update_quote_provider_config", { config: updated });
      setProviderConfig(updated);
      // Recalculate all holding cost bases from scratch with the new setting.
      setRecalculating(true);
      await invoke("recalculate_holdings_cost");
      message.success("持仓成本已根据新设置重新计算");
    } catch (err) {
      message.error("更新失败: " + String(err));
    } finally {
      setRecalculating(false);
    }
  };

  const handleFactoryResetClick = () => {
    setResetAcknowledged(false);
    setResetModalOpen(true);
  };

  const handleConfirmFactoryReset = async () => {
    setResetting(true);
    try {
      // 1. Wipe the SQLite DB and reset backend config tables to defaults.
      await invoke("factory_reset");

      // 2. Drop front-end persisted preferences. The Zustand loaders fall
      //    back to their built-in defaults once the keys are gone.
      LOCAL_STORAGE_KEYS.forEach((key) => localStorage.removeItem(key));

      // 3. Push those defaults into the in-memory stores so the UI updates
      //    immediately, even before the full-page reload below.
      setColorScheme("red-up");
      setThemeMode("system");
      setBaseCurrency("USD");
      setProviderConfig(DEFAULT_PROVIDER_CONFIG);

      setResetModalOpen(false);
      message.success("已恢复出厂设置，应用即将刷新…");
      // Give the toast a moment to render before the page is torn down.
      setTimeout(() => window.location.reload(), 800);
    } catch (err) {
      message.error("恢复出厂设置失败: " + String(err));
    } finally {
      setResetting(false);
    }
  };

  const isXueqiuUsed =
    providerConfig.us_provider === "xueqiu" ||
    providerConfig.hk_provider === "xueqiu" ||
    providerConfig.cn_provider === "xueqiu";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      {isXueqiuUsed && (
        <Card
          title="雪球 Cookie 设置"
          extra={
            <Typography.Text type="secondary" style={{ fontSize: 13 }}>
              Cookie 会过期，届时用任意一种方式重新获取即可
            </Typography.Text>
          }
          styles={{ body: { paddingTop: 12 } }}
        >
          <Tabs
            defaultActiveKey="login"
            items={[
              {
                key: "login",
                label: "一键登录",
                children: (
                  <Space orientation="vertical" style={{ width: "100%" }} size="middle">
                    <Space>
                      <Button onClick={handleOpenLoginWindow}>
                        {loginWindowOpen ? "已打开登录窗口，去登录" : "一键登录雪球"}
                      </Button>
                      <Button
                        type="primary"
                        onClick={handleCapture}
                        loading={capturing}
                        disabled={!loginWindowOpen}
                      >
                        我已登录，抓取 Cookie
                      </Button>
                    </Space>
                    <Typography.Text type="secondary">
                      点击「一键登录雪球」会在独立窗口打开雪球网站，在该窗口内完成登录（扫码 / 账号密码）后，回到本页面点击「我已登录，抓取 Cookie」即可自动获取。Cookie 过期后可重复执行此操作。
                    </Typography.Text>
                  </Space>
                ),
              },
              {
                key: "paste",
                label: "粘贴 Cookie",
                children: (
                  <Form layout="vertical">
                    <Form.Item
                      extra="在浏览器登录 xueqiu.com 后，F12 → Network → 任一请求 → Headers → 复制 Cookie 一整行（含 xq_a_token=...; u=...）"
                    >
                      <Input.TextArea
                        rows={3}
                        placeholder="例如：xq_a_token=6a7dc04b2c6770dc8e...; u=9095890697; ..."
                        value={pastingRaw}
                        onChange={(e) => setPastingRaw(e.target.value)}
                      />
                      <Button
                        style={{ marginTop: 8 }}
                        onClick={handleParsePaste}
                        loading={parsing}
                      >
                        解析并保存
                      </Button>
                    </Form.Item>
                  </Form>
                ),
              },
              {
                key: "manual",
                label: "手动填写",
                children: (
                  <Form layout="vertical">
                    <Form.Item
                      extra="浏览器登录 xueqiu.com → F12 → Application → Cookies → 分别复制 xq_a_token 和 u 的值"
                    >
                      <Space orientation="vertical" style={{ width: "100%" }} size="small">
                        <Input
                          addonBefore="xq_a_token"
                          placeholder="6a7dc04b2c6770dc8e..."
                          value={providerConfig.xueqiu_cookie ?? ""}
                          onChange={(e) =>
                            setProviderConfig({
                              ...providerConfig,
                              xueqiu_cookie: e.target.value || null,
                            })
                          }
                          onBlur={(e) => handleCookieSave(e.target.value)}
                        />
                        <Input
                          addonBefore="u"
                          placeholder="9095890697"
                          value={providerConfig.xueqiu_u ?? ""}
                          onChange={(e) =>
                            setProviderConfig({
                              ...providerConfig,
                              xueqiu_u: e.target.value || null,
                            })
                          }
                          onBlur={(e) => handleUValueSave(e.target.value)}
                        />
                      </Space>
                    </Form.Item>
                  </Form>
                ),
              },
            ]}
          />
        </Card>
      )}

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

      <Card title="外观设置">
        <Form layout="vertical" style={{ maxWidth: 400 }}>
          <Form.Item label="主题模式">
            <Radio.Group
              value={themeMode}
              onChange={(e) => {
                setThemeMode(e.target.value);
                message.success("主题模式已更新");
              }}
            >
              {THEME_OPTIONS.map((opt) => (
                <Radio.Button key={opt.value} value={opt.value}>
                  {opt.icon} {opt.label}
                </Radio.Button>
              ))}
            </Radio.Group>
          </Form.Item>
        </Form>
        <Paragraph type="secondary">
          选择应用的外观主题。跟随系统模式会根据操作系统的设置自动切换深色或浅色主题。
        </Paragraph>
      </Card>

      <Card title="盈亏配色">
        <Form layout="vertical" style={{ maxWidth: 400 }}>
          <Form.Item label="盈亏颜色方案">
            <Radio.Group
              value={colorScheme}
              onChange={(e) => {
                setColorScheme(e.target.value);
                message.success("配色方案已更新");
              }}
            >
              {COLOR_SCHEME_OPTIONS.map((opt) => (
                <Radio.Button key={opt.value} value={opt.value}>
                  {opt.label}
                </Radio.Button>
              ))}
            </Radio.Group>
          </Form.Item>
        </Form>
        <Paragraph type="secondary">
          设置盈亏数值的显示颜色。红涨绿跌为A股习惯（赚钱红色、亏钱绿色），绿涨红跌为欧美习惯（赚钱绿色、亏钱红色）。
        </Paragraph>
      </Card>

      <Card title="持仓成本调整设置">
        <Paragraph>
          买入交易始终会更新持仓均摊成本。卖出和分红是否同步调整均摊成本，可按市场单独设置。
          更改后系统将自动从历史交易记录中重新计算所有持仓成本，请稍候。
        </Paragraph>
        <div style={{ display: "flex", flexDirection: "column", gap: 6, maxWidth: 680 }}>
          <Checkbox
            checked={providerConfig.cn_adjust_sell_pay_cost ?? true}
            disabled={recalculating}
            onChange={(e) =>
              handleCostAdjustChange("cn_adjust_sell_pay_cost", e.target.checked)
            }
          >
            A 股：卖出与分红同步调整持仓均摊成本（默认开启，符合 A 股券商惯例）
          </Checkbox>
          <Checkbox
            checked={providerConfig.us_adjust_sell_pay_cost ?? false}
            disabled={recalculating}
            onChange={(e) =>
              handleCostAdjustChange("us_adjust_sell_pay_cost", e.target.checked)
            }
          >
            美股：卖出与分红同步调整持仓均摊成本（默认关闭，符合 IB 等券商惯例）
          </Checkbox>
          <Checkbox
            checked={providerConfig.hk_adjust_sell_pay_cost ?? false}
            disabled={recalculating}
            onChange={(e) =>
              handleCostAdjustChange("hk_adjust_sell_pay_cost", e.target.checked)
            }
          >
            港股：卖出与分红同步调整持仓均摊成本（默认关闭，符合 IB 等券商惯例）
          </Checkbox>
        </div>
        <Paragraph type="secondary" style={{ marginTop: 12 }}>
          A 股投资收益免税，国内券商通常在卖出或分红后同步调低均摊成本，方便投资者追踪实际持仓成本。
          港股和美股的卖出盈亏需缴所得税、分红需缴红利税，IB 等券商不调整成本，便于准确计算应税收益。
        </Paragraph>
      </Card>

      <Card
        title={<span style={{ color: "var(--color-error)" }}>⚠️ 危险操作</span>}
        styles={{ header: { borderBottomColor: "color-mix(in srgb, var(--color-error) 20%, transparent)" } }}
        style={{ borderColor: "color-mix(in srgb, var(--color-error) 40%, transparent)" }}
      >
        <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            恢复出厂设置将<strong>永久清除</strong>以下数据并重置所有配置：
          </Typography.Paragraph>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            • 所有账户、持仓、交易记录、季度快照、价格提醒<br />
            • 所有 AI 聊天记录、期权记录、股票拆分红利配置<br />
            • 行情数据源与雪球 Cookie、AI 配置、备份配置<br />
            • 盈亏配色、刷新频率、基础货币等界面偏好
          </Typography.Paragraph>
          <Typography.Paragraph type="danger" strong style={{ marginBottom: 0 }}>
            此操作不可恢复，请谨慎操作！
          </Typography.Paragraph>
          <div>
            <Button type="primary" danger onClick={handleFactoryResetClick}>
              恢复出厂设置
            </Button>
          </div>
        </Space>
      </Card>

      <Modal
        open={resetModalOpen}
        title="确认恢复出厂设置"
        okText="确认清空"
        cancelText="取消"
        okButtonProps={{
          danger: true,
          loading: resetting,
          disabled: !resetAcknowledged,
        }}
        cancelButtonProps={{ disabled: resetting }}
        closable={!resetting}
        mask={{ closable: !resetting }}
        onCancel={() => setResetModalOpen(false)}
        onOk={handleConfirmFactoryReset}
      >
        <Space orientation="vertical" size="middle" style={{ width: "100%", paddingTop: 8 }}>
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            即将清空所有账户、持仓、交易、快照、AI 聊天记录等数据，并将全部配置重置为初始状态。
          </Typography.Paragraph>
          <Typography.Paragraph type="danger" strong style={{ marginBottom: 0 }}>
            此操作不可恢复，建议先在「SQLite 备份」页手动备份。
          </Typography.Paragraph>
          <Checkbox
            checked={resetAcknowledged}
            onChange={(e) => setResetAcknowledged(e.target.checked)}
          >
            我已了解此操作会永久清除所有数据且不可恢复
          </Checkbox>
        </Space>
      </Modal>
    </div>
  );
}
