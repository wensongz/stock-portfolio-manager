import { useEffect } from "react";
import { Row, Col, Card, Statistic, Spin, Empty, Select, Tag } from "antd";
import PieChart from "../../components/charts/PieChart";
import HoldingsTable from "../Dashboard/HoldingsTable";
import { useStatisticsStore } from "../../stores/dashboardStore";
import { useCategoryStore } from "../../stores/categoryStore";
import type { CategoryStatistics } from "../../types";
import type { Currency } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const currencySymbol: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  HKD: "HK$",
};

interface Props {
  selectedCategoryId: string;
  onCategoryChange: (id: string) => void;
  baseCurrency: Currency;
}

export default function CategoryTab({ selectedCategoryId, onCategoryChange, baseCurrency }: Props) {
  const { pnlColor } = usePnlColor();
  const { categoryStats, fetchCategoryStats } = useStatisticsStore();
  const { categories, fetchCategories } = useCategoryStore();
  const symbol = currencySymbol[baseCurrency] ?? "$";

  useEffect(() => {
    fetchCategories();
  }, [fetchCategories]);

  useEffect(() => {
    if (selectedCategoryId) {
      fetchCategoryStats(selectedCategoryId, baseCurrency);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedCategoryId, baseCurrency]);

  const stats: CategoryStatistics | undefined = categoryStats[selectedCategoryId];

  return (
    <div>
      <div className="mb-4">
        <Select
          value={selectedCategoryId || undefined}
          onChange={onCategoryChange}
          placeholder="选择投资类别"
          style={{ width: 220 }}
        >
          {categories.map((c) => (
            <Select.Option key={c.id} value={c.id}>
              <Tag color={c.color} style={{ marginRight: 4 }}>
                {c.icon}
              </Tag>
              {c.name}
            </Select.Option>
          ))}
        </Select>
      </div>

      {!selectedCategoryId ? (
        <Empty description="请选择投资类别" />
      ) : !stats ? (
        <div className="flex justify-center py-16">
          <Spin size="large" />
        </div>
      ) : stats.holdings.length === 0 ? (
        <Empty description="该类别暂无持仓" />
      ) : (
        <>
          <Row gutter={[16, 16]} className="mb-4">
            <Col xs={24} sm={8}>
              <Card>
                <Statistic title={`类别总市值 (${baseCurrency})`} value={stats.total_market_value.toFixed(2)} prefix={symbol} />
              </Card>
            </Col>
            <Col xs={24} sm={8}>
              <Card>
                <Statistic title={`类别总成本 (${baseCurrency})`} value={stats.total_cost.toFixed(2)} prefix={symbol} />
              </Card>
            </Col>
            <Col xs={24} sm={8}>
              <Card>
                <Statistic
                  title={`类别总盈亏 (${baseCurrency})`}
                  value={`${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl.toFixed(2)}`}
                  valueStyle={{ color: pnlColor(stats.total_pnl) }}
                  prefix={symbol}
                  suffix={`(${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl_percent.toFixed(2)}%)`}
                />
              </Card>
            </Col>
          </Row>

          {stats.market_distribution.length > 0 && (
            <Row gutter={[16, 16]} className="mb-4">
              <Col xs={24} md={12}>
                <Card title="市场分布">
                  <PieChart data={stats.market_distribution} height={260} currencyCode={baseCurrency} />
                </Card>
              </Col>
            </Row>
          )}

          <Card title="持仓明细">
            <HoldingsTable holdings={stats.holdings} loading={false} />
          </Card>
        </>
      )}
    </div>
  );
}
