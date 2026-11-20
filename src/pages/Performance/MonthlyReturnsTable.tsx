import { Typography } from "antd";
import type { MonthlyReturn } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  data: MonthlyReturn[];
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export default function MonthlyReturnsTable({ data }: Props) {
  const { cellColor, pnlColorDeep } = usePnlColor();
  if (data.length === 0) {
    return <Text type="secondary">暂无月度收益数据</Text>;
  }

  // Build year → month map
  const years = [...new Set(data.map((d) => d.year))].sort();
  const byYear: Record<number, Record<number, MonthlyReturn>> = {};
  for (const d of data) {
    if (!byYear[d.year]) byYear[d.year] = {};
    byYear[d.year][d.month] = d;
  }

  // Compound the selected portion of each year.
  const selectedPeriodReturn: Record<number, number> = {};
  for (const year of years) {
    const months = Object.values(byYear[year]);
    if (months.length === 0) continue;
    const product = months.reduce((acc, m) => acc * (1 + m.return_rate / 100), 1);
    selectedPeriodReturn[year] = (product - 1) * 100;
  }

  return (
    <div>
      <Text strong>📅 月度收益热力图</Text>
      <div className="mt-2 overflow-x-auto">
        <table style={{ borderCollapse: "collapse", fontSize: 12, width: "100%" }}>
          <thead>
            <tr>
              <th style={{ padding: "4px 8px", textAlign: "left" }}>年份</th>
              {MONTHS.map((m) => (
                <th key={m} style={{ padding: "4px 8px", textAlign: "center" }}>
                  {m}
                </th>
              ))}
              <th title="各月份及区间合计按所选日期范围复合计算" style={{ padding: "4px 8px", textAlign: "center" }}>区间合计</th>
            </tr>
          </thead>
          <tbody>
            {years.map((year) => (
              <tr key={year}>
                <td style={{ padding: "4px 8px", fontWeight: "bold" }}>{year}</td>
                {MONTHS.map((_, idx) => {
                  const m = byYear[year]?.[idx + 1];
                  const rate = m?.return_rate ?? null;
                  return (
                    <td
                      key={idx}
                      style={{
                        padding: "4px 8px",
                        textAlign: "center",
                        backgroundColor: rate !== null ? cellColor(rate) : "transparent",
                        color: rate !== null ? pnlColorDeep(rate) : "#aaa",
                        borderRadius: 4,
                      }}
                      title={rate !== null ? `${rate >= 0 ? "+" : ""}${rate.toFixed(2)}%` : "-"}
                    >
                      {rate !== null ? `${rate >= 0 ? "+" : ""}${rate.toFixed(1)}%` : "-"}
                    </td>
                  );
                })}
                <td
                  style={{
                    padding: "4px 8px",
                    textAlign: "center",
                    fontWeight: "bold",
                    backgroundColor: cellColor(selectedPeriodReturn[year] ?? 0),
                    color: (selectedPeriodReturn[year] ?? 0) >= 0 ? pnlColorDeep(1) : pnlColorDeep(-1),
                    borderRadius: 4,
                  }}
                >
                  {selectedPeriodReturn[year] != null
                    ? `${selectedPeriodReturn[year] >= 0 ? "+" : ""}${selectedPeriodReturn[year].toFixed(1)}%`
                    : "-"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
