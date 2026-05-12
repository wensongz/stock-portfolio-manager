import { useState, useMemo } from "react";
import { Button, Space, Table, Tag, Typography } from "antd";
import { EditOutlined, HistoryOutlined } from "@ant-design/icons";
import type { QuarterlyHoldingSnapshot, QuarterlySnapshot } from "../../types";
import HoldingNotesEditor from "./HoldingNotesEditor";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  holdings: QuarterlyHoldingSnapshot[];
  snapshotId: string;
  loading?: boolean;
  snap?: QuarterlySnapshot;
}

function fmtPct(v: number) {
  return `${v >= 0 ? "+" : ""}${v.toFixed(2)}%`;
}

function fmt(v: number) {
  return v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

const MARKET_LABELS: Record<string, string> = {
  US: "🇺🇸 美股",
  CN: "🇨🇳 A股",
  HK: "🇭🇰 港股",
};

const MARKET_CURRENCY_PREFIX: Record<string, string> = {
  US: "$",
  CN: "¥",
  HK: "HK$",
};

export default function SnapshotHoldingsTable({ holdings, snapshotId, loading, snap }: Props) {
  const [notesTarget, setNotesTarget] = useState<QuarterlyHoldingSnapshot | null>(null);
  const [historySymbol, setHistorySymbol] = useState<string | null>(null);
  const [filteredMarkets, setFilteredMarkets] = useState<string[]>([]);
  const [filteredAccountIds, setFilteredAccountIds] = useState<string[]>([]);
  const { pnlColorDark } = usePnlColor();

  // Derive unique accounts from holdings for filter options
  const uniqueAccounts = useMemo(() => {
    const map = new Map<string, { id: string; name: string; market: string }>();
    holdings.forEach((h) => {
      if (!map.has(h.account_id)) {
        map.set(h.account_id, { id: h.account_id, name: h.account_name, market: h.market });
      }
    });
    return [...map.values()];
  }, [holdings]);

  // Rows visible after applying active column filters (used for weight denominator)
  const visibleRows = useMemo(() => {
    return holdings.filter((h) => {
      if (filteredMarkets.length > 0 && !filteredMarkets.includes(h.market)) return false;
      if (filteredAccountIds.length > 0 && !filteredAccountIds.includes(h.account_id)) return false;
      return true;
    });
  }, [holdings, filteredMarkets, filteredAccountIds]);

  // Single active market / account (for currency prefix and weight denominator)
  const singleMarket = filteredMarkets.length === 1 ? filteredMarkets[0] : undefined;
  const singleAccount = filteredAccountIds.length === 1
    ? uniqueAccounts.find((a) => a.id === filteredAccountIds[0])
    : undefined;

  // Weight denominator: account total > market total > 0 (use stored weight)
  const weightDenominator = useMemo(() => {
    if (filteredAccountIds.length > 0) {
      return visibleRows.reduce((sum, h) => sum + h.market_value, 0);
    }
    if (singleMarket && snap) {
      const totals: Record<string, number> = {
        US: snap.us_value,
        CN: snap.cn_value,
        HK: snap.hk_value,
      };
      return totals[singleMarket] ?? 0;
    }
    return 0;
  }, [filteredAccountIds, singleMarket, visibleRows, snap]);

  // Currency prefix for the market_value column header note
  // null = mixed (each row shows its own currency)
  const uniformPrefix: string | null = useMemo(() => {
    if (singleAccount) return MARKET_CURRENCY_PREFIX[singleAccount.market] ?? "";
    if (singleMarket) return MARKET_CURRENCY_PREFIX[singleMarket] ?? "";
    return null;
  }, [singleMarket, singleAccount]);

  function computeWeight(h: QuarterlyHoldingSnapshot): number {
    if ((filteredAccountIds.length > 0 || filteredMarkets.length > 0) && weightDenominator > 0) {
      return (h.market_value / weightDenominator) * 100;
    }
    return h.weight;
  }

  function fmtMv(h: QuarterlyHoldingSnapshot): string {
    const prefix = uniformPrefix ?? (MARKET_CURRENCY_PREFIX[h.market] ?? "");
    return `${prefix}${fmt(h.market_value)}`;
  }

  const weightTitle = filteredAccountIds.length > 0
    ? "仓位% (账户)"
    : filteredMarkets.length > 0
    ? "仓位% (市场)"
    : "仓位% (组合)";

  const marketValueTitle = uniformPrefix !== null
    ? `市值 (${uniformPrefix})`
    : "市值";

  // Column filter options
  const marketFilters = Object.entries(MARKET_LABELS)
    .filter(([k]) => holdings.some((h) => h.market === k))
    .map(([k, v]) => ({ text: v, value: k }));

  const accountFilters = uniqueAccounts.map((a) => ({
    text: `[${MARKET_LABELS[a.market] ?? a.market}] ${a.name}`,
    value: a.id,
  }));

  // Called by antd Table when filters/sorter/pagination change
  function handleTableChange(
    _: unknown,
    filters: Record<string, (string | number | boolean)[] | null>,
  ) {
    setFilteredMarkets((filters.market as string[] | null) ?? []);
    setFilteredAccountIds((filters.account_id as string[] | null) ?? []);
  }

  const columns = [
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      render: (s: string) => <Text strong>{s}</Text>,
      fixed: "left" as const,
    },
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      filters: marketFilters,
      filteredValue: filteredMarkets.length > 0 ? filteredMarkets : null,
      onFilter: (value: unknown, record: QuarterlyHoldingSnapshot) => record.market === value,
      render: (m: string) => <Tag>{MARKET_LABELS[m] ?? m}</Tag>,
    },
    {
      title: "账户",
      dataIndex: "account_id",
      key: "account_id",
      filters: accountFilters,
      filteredValue: filteredAccountIds.length > 0 ? filteredAccountIds : null,
      onFilter: (value: unknown, record: QuarterlyHoldingSnapshot) => record.account_id === value,
      render: (_: string, record: QuarterlyHoldingSnapshot) => record.account_name,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
    },
    {
      title: "类别",
      dataIndex: "category_name",
      key: "category_name",
      render: (name: string, record: QuarterlyHoldingSnapshot) => (
        <Tag color={record.category_color}>{name}</Tag>
      ),
    },
    {
      title: "持股数",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: "均成本",
      dataIndex: "avg_cost",
      key: "avg_cost",
      render: (v: number) => v.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }),
    },
    {
      title: "收盘价",
      dataIndex: "close_price",
      key: "close_price",
      render: (v: number) => fmt(v),
    },
    {
      title: marketValueTitle,
      dataIndex: "market_value",
      key: "market_value",
      render: (_: unknown, record: QuarterlyHoldingSnapshot) => (
        <Text strong>{fmtMv(record)}</Text>
      ),
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) =>
        a.market_value - b.market_value,
    },
    {
      title: weightTitle,
      key: "weight",
      defaultSortOrder: "descend" as const,
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) =>
        computeWeight(a) - computeWeight(b),
      render: (_: unknown, record: QuarterlyHoldingSnapshot) =>
        `${computeWeight(record).toFixed(2)}%`,
    },
    {
      title: "盈亏",
      dataIndex: "pnl",
      key: "pnl",
      render: (v: number) => (
        <Text style={{ color: pnlColorDark(v) }}>
          {v >= 0 ? "+" : ""}{fmt(v)}
        </Text>
      ),
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) => a.pnl - b.pnl,
    },
    {
      title: "盈亏%",
      dataIndex: "pnl_percent",
      key: "pnl_percent",
      render: (v: number) => (
        <Text style={{ color: pnlColorDark(v) }}>{fmtPct(v)}</Text>
      ),
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) =>
        a.pnl_percent - b.pnl_percent,
    },
    {
      title: "操作思考",
      key: "notes",
      render: (_: unknown, record: QuarterlyHoldingSnapshot) => (
        <Space>
          <Button
            size="small"
            icon={<EditOutlined />}
            onClick={() => setNotesTarget(record)}
          >
            {record.notes ? "编辑" : "记录"}
          </Button>
          <Button
            size="small"
            icon={<HistoryOutlined />}
            onClick={() => setHistorySymbol(record.symbol)}
          >
            历史
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Table
        dataSource={holdings}
        columns={columns}
        rowKey="id"
        loading={loading}
        size="small"
        pagination={{ pageSize: 20 }}
        scroll={{ x: "max-content" }}
        onChange={handleTableChange as never}
      />

      {/* Notes editor modal */}
      {notesTarget && (
        <HoldingNotesEditor
          holding={notesTarget}
          snapshotId={snapshotId}
          open={!!notesTarget}
          onClose={() => setNotesTarget(null)}
          showHistory={false}
        />
      )}

      {/* Notes history modal */}
      {historySymbol && (
        <HoldingNotesEditor
          holding={holdings.find((h) => h.symbol === historySymbol) ?? null}
          snapshotId={snapshotId}
          open={!!historySymbol}
          onClose={() => setHistorySymbol(null)}
          showHistory={true}
        />
      )}
    </>
  );
}
