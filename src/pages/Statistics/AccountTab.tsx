import { Alert, Row, Col, Card, Spin, Empty, Select } from "antd";
import PieChart from "../../components/charts/PieChart";
import StatCard from "../../components/charts/StatCard";
import HoldingsTable from "../Dashboard/HoldingsTable";
import { statisticsViewKey, useStatisticsStore } from "../../stores/statisticsStore";
import { useAccountStore } from "../../stores/accountStore";
import type { AccountStatistics } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import StatisticsAiReviewButton from "./StatisticsAiReviewButton";

interface Props {
  selectedAccountId: string;
  onAccountChange: (id: string) => void;
}

const marketCurrency: Record<string, { code: string; symbol: string }> = {
  US: { code: "USD", symbol: "$" },
  CN: { code: "CNY", symbol: "¥" },
  HK: { code: "HKD", symbol: "HK$" },
};

export default function AccountTab({ selectedAccountId, onAccountChange }: Props) {
  const { pnlColor } = usePnlColor();
  const { accountStats, loadingByView, errorByView } = useStatisticsStore();
  const { accounts } = useAccountStore();
  const viewKey = selectedAccountId
    ? statisticsViewKey({ kind: "account", accountId: selectedAccountId })
    : null;
  const loading = viewKey ? (loadingByView[viewKey] ?? false) : false;
  const error = viewKey ? (errorByView[viewKey] ?? null) : null;

  const stats: AccountStatistics | undefined = accountStats[selectedAccountId];
  const selectedAccount = accounts.find((account) => account.id === selectedAccountId);
  const currencyCode = stats ? (marketCurrency[stats.market]?.code ?? "USD") : "USD";
  const currencySymbol = stats ? (marketCurrency[stats.market]?.symbol ?? "$") : "$";

  return (
    <div>
      <div className="mb-4">
        <Select
          value={selectedAccountId || undefined}
          onChange={onAccountChange}
          placeholder="选择账户"
          style={{ width: 220 }}
        >
          {accounts.map((a) => (
            <Select.Option key={a.id} value={a.id}>
              {a.name} ({a.market})
            </Select.Option>
          ))}
        </Select>
      </div>

      {error && <Alert title={error} type="error" showIcon className="mb-4" />}

      {!selectedAccountId ? (
        <Empty description="请选择账户" />
      ) : loading && !stats ? (
        <div className="flex justify-center py-16">
          <Spin size="large" />
        </div>
      ) : !stats ? (
        <Empty description="暂无账户统计数据" />
      ) : stats.holdings.length === 0 ? (
        <Empty description="该账户暂无持仓" />
      ) : (
        <>
          <Row gutter={[16, 16]} className="mb-4">
            <Col xs={24} sm={8}>
              <StatCard title={`账户总市值 (${currencyCode})`} value={stats.total_market_value.toFixed(2)} prefix={currencySymbol} />
            </Col>
            <Col xs={24} sm={8}>
              <StatCard title={`账户总成本 (${currencyCode})`} value={stats.total_cost.toFixed(2)} prefix={currencySymbol} />
            </Col>
            <Col xs={24} sm={8}>
              <StatCard
                title={`账户总盈亏 (${currencyCode})`}
                value={stats.total_pnl.toFixed(2)}
                valueStyle={{ color: pnlColor(stats.total_pnl) }}
                suffix={`(${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl_percent.toFixed(2)}%)`}
              />
            </Col>
          </Row>

          <Row gutter={[16, 16]} className="mb-4">
            {stats.category_distribution.length > 0 && (
              <Col xs={24} md={12}>
                <Card title="类别分布">
                  <PieChart data={stats.category_distribution} height={260} currencyCode={currencyCode} />
                </Card>
              </Col>
            )}
            {stats.stock_distribution.length > 0 && (
              <Col xs={24} md={12}>
                <Card title="个股分布">
                  <PieChart data={stats.stock_distribution} height={260} currencyCode={currencyCode} />
                </Card>
              </Col>
            )}
          </Row>

          <Card
            title="持仓明细"
            extra={(
              <StatisticsAiReviewButton
                scope={{
                  kind: "account",
                  accountId: selectedAccountId,
                  accountName: selectedAccount?.name ?? selectedAccountId,
                }}
              />
            )}
          >
            <HoldingsTable holdings={stats.holdings} loading={loading} hideAccountMarket />
          </Card>
        </>
      )}
    </div>
  );
}
