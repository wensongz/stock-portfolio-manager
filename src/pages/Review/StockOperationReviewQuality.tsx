import { Alert, Collapse, Typography } from "antd";
import type { StockOperationDataQuality } from "../../types";
import { buildStockOperationReviewQualityText } from "./stockOperationReviewViewModel";

const { Text } = Typography;

export default function StockOperationReviewQuality({
  quality,
}: {
  quality: StockOperationDataQuality;
}) {
  return (
    <Alert
      type="info"
      showIcon
      message={buildStockOperationReviewQualityText(quality)}
      description={quality.notes.length > 0 ? (
        <Collapse
          ghost
          size="small"
          items={[{
            key: "notes",
            label: `查看 ${quality.notes.length} 项字段说明`,
            children: (
              <div className="space-y-1">
                {quality.notes.map((note) => (
                  <Text key={note} type="secondary" style={{ display: "block" }}>{note}</Text>
                ))}
              </div>
            ),
          }]}
        />
      ) : undefined}
    />
  );
}
