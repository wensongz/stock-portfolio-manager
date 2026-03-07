import { useEffect } from "react";
import { Card, Row, Col, Statistic, Typography } from "antd";
import { useAccountStore } from "../../stores/accountStore";
import { useHoldingStore } from "../../stores/holdingStore";

const { Title } = Typography;

export default function DashboardPage() {
  const { accounts, fetchAccounts } = useAccountStore();
  const { holdings, fetchHoldings } = useHoldingStore();

  useEffect(() => {
    fetchAccounts();
    fetchHoldings();
  }, [fetchAccounts, fetchHoldings]);

  const usAccounts = accounts.filter((a) => a.market === "US");
  const cnAccounts = accounts.filter((a) => a.market === "CN");
  const hkAccounts = accounts.filter((a) => a.market === "HK");

  return (
    <div>
      <Title level={2}>仪表盘</Title>
      <Row gutter={[16, 16]}>
        <Col span={8}>
          <Card>
            <Statistic title="总持仓数" value={holdings.length} suffix="只" />
          </Card>
        </Col>
        <Col span={8}>
          <Card>
            <Statistic title="证券账户数" value={accounts.length} suffix="个" />
          </Card>
        </Col>
        <Col span={8}>
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
      </Row>

      <Row gutter={[16, 16]} className="mt-4">
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
