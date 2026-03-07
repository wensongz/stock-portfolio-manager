import { useEffect } from "react";
import { Card, Row, Col, Statistic, Typography, Divider, Select, Spin, Alert } from "antd";
import { useAccountStore } from "../../stores/accountStore";
import { useHoldingStore } from "../../stores/holdingStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { useQuoteStore } from "../../stores/quoteStore";
import type { Currency } from "../../types";
import dayjs from "dayjs";

const { Title, Text } = Typography;

export default function DashboardPage() {
  const { accounts, fetchAccounts } = useAccountStore();
  const { holdings, fetchHoldings } = useHoldingStore();
  const { rates, loading: ratesLoading, error: ratesError, baseCurrency, fetchRates, setBaseCurrency, convertWithCachedRates } = useExchangeRateStore();
  const { holdingQuotes, fetchHoldingQuotes } = useQuoteStore();

  useEffect(() => {
    fetchAccounts();
    fetchHoldings();
    fetchRates();
    fetchHoldingQuotes();
  }, [fetchAccounts, fetchHoldings, fetchRates, fetchHoldingQuotes]);

  const usAccounts = accounts.filter((a) => a.market === "US");
  const cnAccounts = accounts.filter((a) => a.market === "CN");
  const hkAccounts = accounts.filter((a) => a.market === "HK");

  // Compute total portfolio values using realtime quotes
  let totalMarketValue = 0;
  let totalCost = 0;
  holdingQuotes.forEach((h) => {
    const currency = h.currency as Currency;
    if (h.market_value !== null && h.market_value !== undefined) {
      totalMarketValue += convertWithCachedRates(h.market_value, currency, baseCurrency);
    }
    if (h.total_cost !== null && h.total_cost !== undefined) {
      totalCost += convertWithCachedRates(h.total_cost, currency, baseCurrency);
    }
  });
  const totalPnl = totalMarketValue - totalCost;
  const totalPnlPercent = totalCost > 0 ? (totalPnl / totalCost) * 100 : 0;
  const hasQuotes = holdingQuotes.some((h) => h.quote !== null);

  const currencySymbol: Record<Currency, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">仪表盘</Title>
        <div className="flex items-center gap-2">
          <Text type="secondary">基准货币:</Text>
          <Select
            value={baseCurrency}
            onChange={setBaseCurrency}
            size="small"
            style={{ width: 100 }}
          >
            <Select.Option value="USD">USD 美元</Select.Option>
            <Select.Option value="CNY">CNY 人民币</Select.Option>
            <Select.Option value="HKD">HKD 港元</Select.Option>
          </Select>
        </div>
      </div>

      <Row gutter={[16, 16]}>
        <Col span={6}>
          <Card>
            <Statistic title="总持仓数" value={holdings.length} suffix="只" />
          </Card>
        </Col>
        <Col span={6}>
          <Card>
            <Statistic title="证券账户数" value={accounts.length} suffix="个" />
          </Card>
        </Col>
        {hasQuotes && (
          <>
            <Col span={6}>
              <Card>
                <Statistic
                  title={`总市值 (${baseCurrency})`}
                  value={totalMarketValue.toFixed(2)}
                  prefix={currencySymbol[baseCurrency]}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card>
                <Statistic
                  title={`总盈亏 (${baseCurrency})`}
                  value={`${totalPnl >= 0 ? "+" : ""}${totalPnl.toFixed(2)} (${totalPnlPercent >= 0 ? "+" : ""}${totalPnlPercent.toFixed(2)}%)`}
                  valueStyle={{ color: totalPnl >= 0 ? "#3f8600" : "#cf1322" }}
                />
              </Card>
            </Col>
          </>
        )}
        {!hasQuotes && (
          <Col span={12}>
            <Card>
              <Statistic
                title="覆盖市场"
                value={[
                  usAccounts.length > 0 ? "🇺🇸 US" : "",
                  cnAccounts.length > 0 ? "🇨🇳 CN" : "",
                  hkAccounts.length > 0 ? "🇭🇰 HK" : "",
                ]
                  .filter(Boolean)
                  .join(" / ") || "—"}
              />
            </Card>
          </Col>
        )}
      </Row>

      {/* Exchange Rates Card */}
      <Row gutter={[16, 16]} className="mt-4">
        <Col span={24}>
          <Card
            title="💱 实时汇率"
            extra={
              ratesLoading ? (
                <Spin size="small" />
              ) : rates ? (
                <Text type="secondary" style={{ fontSize: 12 }}>
                  更新于 {dayjs(rates.updated_at).format("YYYY-MM-DD HH:mm")}
                </Text>
              ) : null
            }
          >
            {ratesError && (
              <Alert
                message="汇率获取失败，请检查网络连接"
                type="warning"
                showIcon
                style={{ marginBottom: 16 }}
              />
            )}
            {rates ? (
              <Row gutter={[32, 0]}>
                <Col>
                  <Statistic title="USD / CNY" value={rates.usd_cny.toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="USD / HKD" value={rates.usd_hkd.toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="CNY / HKD" value={rates.cny_hkd.toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="CNY / USD" value={(1 / rates.usd_cny).toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="HKD / USD" value={(1 / rates.usd_hkd).toFixed(4)} />
                </Col>
              </Row>
            ) : (
              !ratesLoading && <Text type="secondary">暂无汇率数据</Text>
            )}
          </Card>
        </Col>
      </Row>

      <Divider />

      <Row gutter={[16, 16]}>
        <Col span={8}>
          <Card title="🇺🇸 美股账户" size="small">
            {usAccounts.length === 0 ? (
              <p className="text-gray-400">暂无账户</p>
            ) : (
              usAccounts.map((a) => (
                <div key={a.id} className="py-1 border-b last:border-0">
                  {a.name}
                </div>
              ))
            )}
          </Card>
        </Col>
        <Col span={8}>
          <Card title="🇨🇳 A股账户" size="small">
            {cnAccounts.length === 0 ? (
              <p className="text-gray-400">暂无账户</p>
            ) : (
              cnAccounts.map((a) => (
                <div key={a.id} className="py-1 border-b last:border-0">
                  {a.name}
                </div>
              ))
            )}
          </Card>
        </Col>
        <Col span={8}>
          <Card title="🇭🇰 港股账户" size="small">
            {hkAccounts.length === 0 ? (
              <p className="text-gray-400">暂无账户</p>
            ) : (
              hkAccounts.map((a) => (
                <div key={a.id} className="py-1 border-b last:border-0">
                  {a.name}
                </div>
              ))
            )}
          </Card>
        </Col>
      </Row>
    </div>
  );
}
