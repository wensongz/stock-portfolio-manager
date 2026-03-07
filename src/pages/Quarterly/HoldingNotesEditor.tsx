import { useEffect, useState } from "react";
import { Button, Descriptions, Divider, Modal, Space, Spin, Tag, Timeline, Typography } from "antd";
import { invoke } from "@tauri-apps/api/core";
import MDEditor from "@uiw/react-md-editor";
import type { HoldingNoteHistory, QuarterlyHoldingSnapshot } from "../../types";
import { useQuarterlyStore } from "../../stores/quarterlyStore";

const { Text } = Typography;

interface Props {
  holding: QuarterlyHoldingSnapshot | null;
  snapshotId: string;
  open: boolean;
  onClose: () => void;
  showHistory: boolean;
}

const NOTE_TEMPLATE = `### 买入/卖出/持有理由
- 

### 当前估值判断
- 

### 后续计划
- 

### 风险提示
- `;

export default function HoldingNotesEditor({
  holding,
  snapshotId,
  open,
  onClose,
  showHistory,
}: Props) {
  const { updateHoldingNotes } = useQuarterlyStore();
  const [notes, setNotes] = useState(holding?.notes ?? "");
  const [history, setHistory] = useState<HoldingNoteHistory[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [mode, setMode] = useState<"edit" | "history">(showHistory ? "history" : "edit");

  useEffect(() => {
    if (holding) {
      setNotes(holding.notes ?? "");
    }
  }, [holding]);

  useEffect(() => {
    if (open && holding && (showHistory || mode === "history")) {
      fetchHistory();
    }
  }, [open, holding, mode]);

  const fetchHistory = async () => {
    if (!holding) return;
    setHistoryLoading(true);
    try {
      const data = await invoke<HoldingNoteHistory[]>("get_holding_notes_history", {
        symbol: holding.symbol,
      });
      setHistory(data);
    } catch (e) {
      console.error(e);
    } finally {
      setHistoryLoading(false);
    }
  };

  const handleSave = async () => {
    if (!holding) return;
    setSaving(true);
    await updateHoldingNotes(snapshotId, holding.symbol, notes);
    setSaving(false);
    onClose();
  };

  if (!holding) return null;

  return (
    <Modal
      open={open}
      onCancel={onClose}
      width={800}
      title={
        <Space>
          <Text strong>{holding.symbol}</Text>
          <Text type="secondary">{holding.name}</Text>
          <Tag>{holding.market}</Tag>
        </Space>
      }
      footer={
        mode === "edit" ? (
          <Space>
            <Button onClick={onClose}>取消</Button>
            <Button type="primary" loading={saving} onClick={handleSave}>
              保存
            </Button>
          </Space>
        ) : (
          <Button onClick={onClose}>关闭</Button>
        )
      }
    >
      <Space className="mb-3">
        <Button
          type={mode === "edit" ? "primary" : "default"}
          size="small"
          onClick={() => setMode("edit")}
        >
          编辑思考
        </Button>
        <Button
          type={mode === "history" ? "primary" : "default"}
          size="small"
          onClick={() => setMode("history")}
        >
          历史时间线
        </Button>
      </Space>

      <Descriptions size="small" column={3} className="mb-3">
        <Descriptions.Item label="持股数">{holding.shares.toLocaleString()}</Descriptions.Item>
        <Descriptions.Item label="均成本">{holding.avg_cost.toFixed(4)}</Descriptions.Item>
        <Descriptions.Item label="收盘价">{holding.close_price.toFixed(4)}</Descriptions.Item>
        <Descriptions.Item label="盈亏%">
          <Text style={{ color: holding.pnl_percent >= 0 ? "#3f8600" : "#cf1322" }}>
            {holding.pnl_percent >= 0 ? "+" : ""}
            {holding.pnl_percent.toFixed(2)}%
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label="类别">{holding.category_name}</Descriptions.Item>
        <Descriptions.Item label="仓位">{holding.weight.toFixed(2)}%</Descriptions.Item>
      </Descriptions>

      <Divider />

      {mode === "edit" && (
        <>
          {!notes && (
            <Button
              size="small"
              className="mb-2"
              onClick={() => setNotes(NOTE_TEMPLATE)}
            >
              使用模板
            </Button>
          )}
          <div data-color-mode="light">
            <MDEditor
              value={notes}
              onChange={(v) => setNotes(v ?? "")}
              height={300}
              preview="edit"
            />
          </div>
        </>
      )}

      {mode === "history" && (
        <>
          {historyLoading ? (
            <Spin />
          ) : history.length === 0 ? (
            <Text type="secondary">暂无历史思考记录</Text>
          ) : (
            <Timeline
              items={history.map((h) => ({
                label: h.quarter,
                children: (
                  <div>
                    <div className="text-xs text-gray-500 mb-1">
                      {h.snapshot_date} | 持股: {h.shares} | 成本: {h.avg_cost.toFixed(4)} |
                      收盘: {h.close_price.toFixed(4)} |{" "}
                      <span style={{ color: h.pnl_percent >= 0 ? "#3f8600" : "#cf1322" }}>
                        {h.pnl_percent >= 0 ? "+" : ""}
                        {h.pnl_percent.toFixed(2)}%
                      </span>
                    </div>
                    {h.notes ? (
                      <div data-color-mode="light">
                        <MDEditor.Markdown source={h.notes} style={{ background: "transparent" }} />
                      </div>
                    ) : (
                      <Text type="secondary">（无思考记录）</Text>
                    )}
                  </div>
                ),
              }))}
              mode="left"
            />
          )}
        </>
      )}
    </Modal>
  );
}
