import { useState, useEffect } from "react";
import { Button, Space, Typography, message } from "antd";
import MDEditor, { commands } from "@uiw/react-md-editor";
import type { ICommand } from "@uiw/react-md-editor";
import { useQuarterlyStore } from "../../stores/quarterlyStore";

const { Text } = Typography;

interface Props {
  snapshotId: string;
  initialNotes: string;
}

const MARKET_SECTION = `### 本季度回顾
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

const NOTE_TEMPLATE = `## 🇨🇳 A股

${MARKET_SECTION}
---

## 🇭🇰 港股

${MARKET_SECTION}
---

## 🇺🇸 美股

${MARKET_SECTION}`;

const INDENT = "  "; // 2 spaces

const indentCommand: ICommand = {
  name: "indent",
  keyCommand: "indent",
  buttonProps: { "aria-label": "增加缩进", title: "增加缩进" },
  icon: (
    <svg width="12" height="12" viewBox="0 0 24 24">
      <path
        fill="currentColor"
        d="M3 21h18v-2H3v2zm8-4h10v-2H11v2zm-8-4h18v-2H3v2zm8-4h10V7H11v2zM3 3v2h18V3H3zm8 8l5-5-5-5v10z"
      />
    </svg>
  ),
  execute: (state, api) => {
    const { text, selection } = state;
    const lineStart = text.lastIndexOf("\n", selection.start - 1) + 1;
    const region = text.slice(lineStart, selection.end);
    const newRegion = region.replace(/^/gm, INDENT);
    const firstLineDelta = newRegion.split("\n")[0].length - region.split("\n")[0].length;
    const totalDelta = newRegion.length - region.length;
    api.setSelectionRange({ start: lineStart, end: selection.end });
    api.replaceSelection(newRegion);
    api.setSelectionRange({
      start: Math.max(lineStart, selection.start + firstLineDelta),
      end: selection.end + totalDelta,
    });
  },
};

const unindentCommand: ICommand = {
  name: "unindent",
  keyCommand: "unindent",
  buttonProps: { "aria-label": "减少缩进", title: "减少缩进" },
  icon: (
    <svg width="12" height="12" viewBox="0 0 24 24">
      <path
        fill="currentColor"
        d="M11 17h10v-2H11v2zm-8-5l5 5V7l-5 5zm0 9h18v-2H3v2zM3 3v2h18V3H3zm8 4h10V5H11v2zm0 4h10v-2H11v2z"
      />
    </svg>
  ),
  execute: (state, api) => {
    const { text, selection } = state;
    const lineStart = text.lastIndexOf("\n", selection.start - 1) + 1;
    const region = text.slice(lineStart, selection.end);
    const newRegion = region.replace(/^  /gm, "");
    const firstLineDelta = newRegion.split("\n")[0].length - region.split("\n")[0].length;
    const totalDelta = newRegion.length - region.length;
    api.setSelectionRange({ start: lineStart, end: selection.end });
    api.replaceSelection(newRegion);
    api.setSelectionRange({
      start: Math.max(lineStart, selection.start + firstLineDelta),
      end: selection.end + totalDelta,
    });
  },
};

const TOOLBAR_COMMANDS = [...commands.getCommands(), commands.divider, indentCommand, unindentCommand];

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
        <MDEditor
          value={notes}
          onChange={(v) => setNotes(v ?? "")}
          height={350}
          commands={TOOLBAR_COMMANDS}
        />
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
