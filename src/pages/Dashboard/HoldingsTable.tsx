import { useMemo, useState } from "react";
import { Table, Tag, Typography } from "antd";
import type { ColumnsType, TableProps } from "antd/es/table";
import type { HoldingDetail } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  holdings: HoldingDetail[];
  loading: boolean;
  hideAccountMarket?: boolean;
}

const marketMeta: Record<string, { emoji: string; name: string }> = {
  US: { emoji: "🇺🇸", name: "美股" },
  CN: { emoji: "🇨🇳", name: "A股" },
  HK: { emoji: "🇭🇰", name: "港股" },
};

const isWindows = navigator.userAgent.includes("Windows");

function starPoints(cx: number, cy: number, radius: number, rotation = -90) {
  const innerRadius = radius * 0.381966;
  return Array.from({ length: 10 }, (_, index) => {
    const angle = ((rotation + index * 36) * Math.PI) / 180;
    const r = index % 2 === 0 ? radius : innerRadius;
    return `${cx + Math.cos(angle) * r},${cy + Math.sin(angle) * r}`;
  }).join(" ");
}

const HK_PETALS = [
  "M269.28907 253.51795 C224.42570 272.48031 244.31386 330.91058 287.21820 327.97219 C278.53937 323.51074 277.28164 315.18850 282.23915 307.47713 C287.78939 298.84365 281.72494 284.95304 274.22249 281.57669 C261.88894 276.02646 259.42224 261.22649 269.28907 253.51795 Z",
  "M269.28907 253.51795 C237.39165 216.71036 187.96734 253.68009 204.01937 293.57603 C205.58098 283.94334 213.10696 280.17609 221.97317 282.50787 C231.89896 285.11802 243.23584 275.05899 244.12847 266.88019 C245.59569 253.43546 258.90917 246.51609 269.28907 253.51795 Z",
  "M269.28907 253.51795 C294.43833 211.80756 244.00630 176.22652 211.02350 203.82123 C220.66639 202.32992 226.57521 208.32321 227.09707 217.47600 C227.68157 227.72268 240.75184 235.39606 248.80620 233.71767 C262.04627 230.95843 272.74054 241.48233 269.28907 253.51795 Z",
  "M269.28907 253.51795 C316.72828 264.54728 334.98397 205.58891 298.54828 182.74762 C302.94680 191.45792 299.07269 198.92948 290.52935 202.25395 C280.96469 205.97613 277.70542 220.77780 281.79099 227.91940 C288.50570 239.65852 281.80205 253.08142 269.28907 253.51795 Z",
  "M269.28907 253.51795 C273.45855 302.04340 335.17191 301.18706 345.63600 259.47638 C338.71096 266.35096 330.40828 264.97502 324.60661 257.87735 C318.11102 249.93099 303.02731 251.40586 297.49805 257.49694 C288.40819 267.51175 273.57080 265.28372 269.28907 253.51795 Z",
];

const HK_STARS = [
  "M266.86375 295.71279 L264.33865 291.75194 263.42391 296.35880 258.87685 297.53575 262.97518 299.83011 262.68945 304.51805 266.13780 301.32964 270.50769 303.05027 268.54101 298.78526 271.52787 295.16060 Z",
  "M228.41065 264.24964 L231.39694 260.62498 226.73282 261.17773 224.20857 257.21688 223.29241 261.82318 218.74620 263.00041 222.84425 265.29506 222.55795 269.98299 226.00687 266.79430 230.37676 268.51465 Z",
  "M246.45061 217.95591 L250.82135 219.67625 248.85439 215.41153 251.84069 211.78715 247.17657 212.33991 244.65288 208.37906 243.73672 212.98535 239.18995 214.16202 243.28800 216.45638 243.00227 221.14460 Z",
  "M296.05266 220.80898 L295.76693 225.49663 299.21584 222.30822 303.58545 224.02828 301.61906 219.76356 304.60535 216.13918 299.94180 216.69137 297.41726 212.73137 296.50139 217.33739 291.95490 218.51405 Z",
  "M308.66740 268.86416 L304.12063 270.04110 308.21811 272.33546 307.93266 277.02340 311.38101 273.83499 315.75061 275.55506 313.78422 271.29061 316.77080 267.66595 312.10696 268.21814 309.58356 264.25786 Z",
];

const HK_STYLES = [
  "M269.54249 254.01175 C264.11556 256.79622 259.74028 262.60101 257.83739 269.53852 C255.65471 277.49792 256.90904 286.04891 261.27978 292.99805 L260.33981 293.58879 C255.80409 286.37688 254.50243 277.50387 256.76702 269.24513 C258.77991 261.90482 263.25184 255.99203 269.03565 253.02387 Z",
  "M268.89789 253.91169 C264.57222 249.61068 257.69962 247.24261 250.51408 247.57710 C242.26951 247.96091 234.52526 251.79619 229.26643 258.10016 L228.41405 257.38923 C233.87187 250.84658 241.90809 246.86617 250.46249 246.46819 C258.06529 246.11471 265.07027 248.54060 269.68054 253.12450 Z",
  "M268.79386 253.26765 C271.54715 247.82485 271.67556 240.55654 269.13713 233.82567 C266.22510 226.10409 260.18391 219.92372 252.56381 216.87024 L252.97710 215.83984 C260.88520 219.00869 267.15373 225.42180 270.17575 233.43392 C272.86186 240.55625 272.71928 247.96772 269.78457 253.76854 Z",
  "M269.37439 252.96973 C275.40170 253.90630 282.35339 251.78258 287.97024 247.28854 C294.41452 242.13231 298.42526 234.47764 298.97461 226.28721 L300.08183 226.36120 C299.51206 234.86173 295.34995 242.80526 288.66387 248.15509 C282.72076 252.91020 275.62791 255.06482 269.20403 254.06646 Z",
  "M269.83672 253.42951 C270.80816 259.45115 274.97594 265.40617 280.98567 269.35937 C287.88066 273.89509 296.40047 275.34416 304.35987 273.33581 L304.63115 274.41213 C296.37043 276.49644 287.52973 274.99266 280.37594 270.28658 C274.01698 266.10406 269.77635 260.02403 268.74113 253.60639 Z",
];

function MarketFlag({ market }: { market: string }) {
  const commonProps = {
    width: 20,
    height: 14,
    style: { verticalAlign: "-2px", borderRadius: 2, boxShadow: "0 0 0 1px rgba(0,0,0,.12)", overflow: "hidden" },
    "aria-hidden": true,
  } as const;

  if (market === "US") {
    return (
      <svg {...commonProps} viewBox="0 0 30 20">
        <rect width="30" height="20" fill="#fff" />
        {Array.from({ length: 7 }, (_, index) => (
          <rect key={index} y={(index * 40) / 13} width="30" height={20 / 13} fill="#b22234" />
        ))}
        <rect width="12" height={140 / 13} fill="#3c3b6e" />
        {Array.from({ length: 9 }, (_, row) => {
          const count = row % 2 === 0 ? 6 : 5;
          return Array.from({ length: count }, (__, column) => (
            <polygon
              key={`${row}-${column}`}
              points={starPoints(row % 2 === 0 ? 1 + column * 2 : 2 + column * 2, 0.6 + row * 1.2, 0.38)}
              fill="#fff"
            />
          ));
        })}
      </svg>
    );
  }
  if (market === "CN") {
    return (
      <svg {...commonProps} viewBox="0 0 30 20">
        <rect width="30" height="20" fill="#de2910" />
        <polygon points={starPoints(5, 5, 3)} fill="#ffde00" />
        {[[10, 2], [12, 4], [12, 7], [10, 9]].map(([x, y]) => (
          <polygon
            key={`${x}-${y}`}
            points={starPoints(x, y, 1, Math.atan2(5 - y, 5 - x) * 180 / Math.PI)}
            fill="#ffde00"
          />
        ))}
      </svg>
    );
  }
  if (market === "HK") {
    return (
      <svg {...commonProps} viewBox="82.59817 128.29380 381.34630 254.23086">
        <g transform="translate(0 510.81846) scale(1 -1)">
          <rect x="82.59817" y="128.29380" width="381.34630" height="254.23086" fill="#de2910" />
          {HK_PETALS.map((d) => <path key={d} d={d} fill="#fff" />)}
          {HK_STARS.map((d) => <path key={d} d={d} fill="#de2910" />)}
          {HK_STYLES.map((d) => <path key={d} d={d} fill="#de2910" />)}
        </g>
      </svg>
    );
  }
  return null;
}

function MarketLabel({ market, useName = false }: { market: string; useName?: boolean }) {
  const meta = marketMeta[market];
  if (!meta) return <>{market}</>;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      {isWindows ? <MarketFlag market={market} /> : <span>{meta.emoji}</span>}
      <span>{useName ? meta.name : market}</span>
    </span>
  );
}

const currencySymbol: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

function fmtMoney(value: number, currency: string, fractionDigits = 2) {
  return `${currencySymbol[currency] ?? ""}${value.toLocaleString("en-US", {
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  })}`;
}

export default function HoldingsTable({ holdings, loading, hideAccountMarket = false }: Props) {
  const { pnlColor } = usePnlColor();

  // Track which filter values are currently active for account and market columns.
  // This lets us recompute the denominator whenever holdings or filters change.
  const [activeAccountFilter, setActiveAccountFilter] = useState<string[] | null>(null);
  const [activeMarketFilter, setActiveMarketFilter] = useState<string[] | null>(null);

  const filteredTotalMvUsd = useMemo(() => {
    const visible = holdings.filter((h) => {
      if (activeAccountFilter && activeAccountFilter.length > 0 && !activeAccountFilter.includes(h.account_name))
        return false;
      if (activeMarketFilter && activeMarketFilter.length > 0 && !activeMarketFilter.includes(h.market))
        return false;
      return true;
    });
    return visible.reduce((sum, h) => sum + h.market_value_usd, 0);
  }, [holdings, activeAccountFilter, activeMarketFilter]);

  const handleTableChange: TableProps<HoldingDetail>["onChange"] = (_pagination, filters) => {
    const accountVals = filters["account_name"];
    const marketVals = filters["market"];
    setActiveAccountFilter(accountVals ? (accountVals as string[]) : null);
    setActiveMarketFilter(marketVals ? (marketVals as string[]) : null);
  };

  const accountFilters = useMemo(
    () =>
      Array.from(new Set(holdings.map((h) => h.account_name))).map((name) => ({
        text: name,
        value: name,
      })),
    [holdings]
  );

  const columns: ColumnsType<HoldingDetail> = useMemo(() => {
    const allColumns: ColumnsType<HoldingDetail> = [
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      sorter: (a, b) => a.symbol.localeCompare(b.symbol),
      render: (symbol: string) => <Text strong>{symbol}</Text>,
      fixed: "left",
      width: 100,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      ellipsis: true,
      width: 140,
    },
    {
      title: "账户",
      dataIndex: "account_name",
      key: "account_name",
      filters: accountFilters,
      onFilter: (value, record) => record.account_name === value,
      ellipsis: true,
      width: 120,
    },
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      render: (market: string) => <MarketLabel market={market} />,
      filters: [
        { text: <MarketLabel market="US" useName />, value: "US" },
        { text: <MarketLabel market="CN" useName />, value: "CN" },
        { text: <MarketLabel market="HK" useName />, value: "HK" },
      ],
      onFilter: (value, record) => record.market === value,
      width: 80,
    },
    {
      title: "类别",
      dataIndex: "category_name",
      key: "category_name",
      sorter: (a, b) => a.category_name.localeCompare(b.category_name),
      render: (name: string, record: HoldingDetail) => (
        <Tag color={record.category_color}>{name}</Tag>
      ),
      width: 60,
    },
    {
      title: "持仓数量",
      dataIndex: "shares",
      key: "shares",
      sorter: (a, b) => a.shares - b.shares,
      render: (shares: number) => shares.toLocaleString(),
      align: "right",
      width: 90,
    },
    {
      title: "均价",
      dataIndex: "avg_cost",
      key: "avg_cost",
      sorter: (a, b) => a.avg_cost - b.avg_cost,
      render: (price: number, record: HoldingDetail) =>
        fmtMoney(price, record.currency, 3),
      align: "right",
      width: 90,
    },
    {
      title: "现价",
      dataIndex: "current_price",
      key: "current_price",
      sorter: (a, b) => a.current_price - b.current_price,
      render: (price: number, record: HoldingDetail) =>
        fmtMoney(price, record.currency),
      align: "right",
      width: 90,
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      defaultSortOrder: "descend" as const,
      render: (value: number, record: HoldingDetail) =>
        fmtMoney(value, record.currency),
      align: "right",
      width: 140,
    },
    {
      title: "仓位%",
      key: "position_pct",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      render: (_: unknown, record: HoldingDetail) => {
        const pct = filteredTotalMvUsd > 0 ? (record.market_value_usd / filteredTotalMvUsd) * 100 : 0;
        return `${pct.toFixed(2)}%`;
      },
      align: "right",
      width: 70,
    },
    {
      title: "盈亏金额",
      dataIndex: "pnl",
      key: "pnl",
      sorter: (a, b) => a.pnl - b.pnl,
      render: (pnl: number, record: HoldingDetail) => (
        <span style={{ color: pnlColor(pnl) }}>
          {pnl >= 0 ? "+" : "-"}
          {fmtMoney(Math.abs(pnl), record.currency)}
        </span>
      ),
      align: "right",
      width: 140,
    },
    {
      title: "盈亏比例",
      dataIndex: "pnl_percent",
      key: "pnl_percent",
      render: (pnl: number | null) =>
        pnl != null ? (
          <span style={{ color: pnlColor(pnl) }}>
            {pnl >= 0 ? "+" : ""}
            {pnl.toFixed(2)}%
          </span>
        ) : (
          <span>-</span>
        ),
      align: "right",
      width: 80,
    },
    ];
    return hideAccountMarket
      ? allColumns.filter((c) => c.key !== "account_name" && c.key !== "market")
      : allColumns;
  }, [accountFilters, filteredTotalMvUsd, pnlColor, hideAccountMarket]);

  return (
    <Table<HoldingDetail>
      columns={columns}
      dataSource={holdings}
      rowKey="id"
      loading={loading}
      scroll={{ x: hideAccountMarket ? 1100 : 1310 }}
      size="small"
      pagination={{ pageSize: 20, showSizeChanger: true }}
      bordered
      onChange={handleTableChange}
    />
  );
}
