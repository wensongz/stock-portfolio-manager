import { Typography } from "antd";
import type { MonthlyReturn } from "../../types";

const { Text } = Typography;

const SATURATION_THRESHOLD = 10; // % at which color is fully saturated

interface Props {
  data: MonthlyReturn[];
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function cellColor(rate: number): string {
  if (rate === 0) return "transparent";
  const intensity = Math.min(Math.abs(rate) / SATURATION_THRESHOLD, 1);
  if (rate > 0) {
    const g = Math.round(80 + intensity * 100);
    return `rgba(0, ${g}, 0, ${0.15 + intensity * 0.5})`;
  }
  const r = Math.round(150 + intensity * 100);
  return `rgba(${r}, 0, 0, ${0.15 + intensity * 0.5})`;
}

export default function MonthlyReturnsTable({ data }: Props) {
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

  // Annual return per year
  const annualReturn: Record<number, number> = {};
  for (const year of years) {
    const months = Object.values(byYear[year]);
    if (months.length === 0) continue;
    const product = months.reduce((acc, m) => acc * (1 + m.return_rate / 100), 1);
    annualReturn[year] = (product - 1) * 100;
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
              <th style={{ padding: "4px 8px", textAlign: "center" }}>全年</th>
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
                        color: rate !== null ? (rate >= 0 ? "#135200" : "#820014") : "#aaa",
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
                    backgroundColor: cellColor(annualReturn[year] ?? 0),
                    color: (annualReturn[year] ?? 0) >= 0 ? "#135200" : "#820014",
                    borderRadius: 4,
                  }}
                >
                  {annualReturn[year] != null
                    ? `${annualReturn[year] >= 0 ? "+" : ""}${annualReturn[year].toFixed(1)}%`
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
