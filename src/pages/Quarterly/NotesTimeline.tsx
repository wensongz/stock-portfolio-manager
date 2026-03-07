import { useEffect } from "react";
import { Card, Empty, Timeline, Typography } from "antd";
import MDEditor from "@uiw/react-md-editor";
import { useQuarterlyStore } from "../../stores/quarterlyStore";

const { Text, Title } = Typography;

function fmt(v: number) {
  return v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export default function NotesTimeline() {
  const { notesSummaries, loading, fetchNotesSummaries } = useQuarterlyStore();

  useEffect(() => {
    fetchNotesSummaries();
  }, []);

  const withNotes = notesSummaries.filter((n) => n.overall_notes.trim().length > 0);

  return (
    <Card size="small" title={<Title level={4} className="!mb-0">📝 季度总结时间线</Title>} loading={loading}>
      {withNotes.length === 0 ? (
        <Empty description="暂无季度总结" />
      ) : (
        <Timeline
          mode="left"
          items={withNotes.map((n) => ({
            label: (
              <div className="text-right">
                <Text strong>{n.quarter}</Text>
                <br />
                <Text type="secondary" className="text-xs">{n.snapshot_date}</Text>
              </div>
            ),
            children: (
              <Card size="small" className="mb-2">
                <div className="flex gap-4 mb-2 text-sm text-gray-500">
                  <span>总市值: ${fmt(n.total_value)}</span>
                  <span style={{ color: n.total_pnl >= 0 ? "#3f8600" : "#cf1322" }}>
                    盈亏: {n.total_pnl >= 0 ? "+" : ""}${fmt(n.total_pnl)}
                  </span>
                </div>
                <div data-color-mode="light">
                  <MDEditor.Markdown
                    source={n.overall_notes}
                    style={{ background: "transparent", fontSize: 13 }}
                  />
                </div>
              </Card>
            ),
          }))}
        />
      )}
    </Card>
  );
}
