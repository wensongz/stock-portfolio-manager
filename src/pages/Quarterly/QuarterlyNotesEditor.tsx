import { useState, useEffect } from "react";
import { Button, Space, Typography, message } from "antd";
import MDEditor from "@uiw/react-md-editor";
import { useQuarterlyStore } from "../../stores/quarterlyStore";

const { Text } = Typography;

interface Props {
  snapshotId: string;
  initialNotes: string;
}

const NOTE_TEMPLATE = `### 本季度回顾
- 整体市场环境：
- 主要操作：
- 收益来源：
- 亏损来源：

### 经验教训
- 

### 下季度计划
- 关注标的：
- 仓位调整计划：
- 风险控制：
`;

export default function QuarterlyNotesEditor({ snapshotId, initialNotes }: Props) {
  const { updateQuarterlyNotes } = useQuarterlyStore();
  const [notes, setNotes] = useState(initialNotes);
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!editing) {
      setNotes(initialNotes);
    }
  }, [initialNotes, editing]);

  const handleSave = async () => {
    setSaving(true);
    try {
      await updateQuarterlyNotes(snapshotId, notes);
      setEditing(false);
    } catch (err) {
      message.error("保存失败: " + String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleCancel = () => {
    setNotes(initialNotes);
    setEditing(false);
  };

  if (!editing) {
    return (
      <div>
        {notes ? (
          <div data-color-mode="light">
            <MDEditor.Markdown source={notes} style={{ background: "transparent" }} />
          </div>
        ) : (
          <Text type="secondary">尚未填写季度总结</Text>
        )}
        <div className="mt-3">
          <Button size="small" onClick={() => setEditing(true)}>
            {notes ? "编辑总结" : "写季度总结"}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div>
      {!notes && (
        <Button size="small" className="mb-2" onClick={() => setNotes(NOTE_TEMPLATE)}>
          使用模板
        </Button>
      )}
      <div data-color-mode="light">
        <MDEditor value={notes} onChange={(v) => setNotes(v ?? "")} height={350} />
      </div>
      <Space className="mt-3">
        <Button onClick={handleCancel}>取消</Button>
        <Button type="primary" loading={saving} onClick={handleSave}>
          保存
        </Button>
      </Space>
    </div>
  );
}
