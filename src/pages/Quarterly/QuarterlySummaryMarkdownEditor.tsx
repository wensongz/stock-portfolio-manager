import MDEditor, { commands } from "@uiw/react-md-editor";
import type { ICommand } from "@uiw/react-md-editor";

const INDENT = "  "; // 2 spaces

const indentCommand: ICommand = {
  name: "indent",
  keyCommand: "indent",
  buttonProps: { "aria-label": "增加缩进", title: "增加缩进" },
  icon: (
    <svg width="12" height="12" viewBox="0 0 24 24">
      <path
        fill="currentColor"
        d="M3 8v8l6-4-6-4zM11 4h10v3H11zM14 11h7v3h-7zM11 17h10v3H11z"
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
        d="M8 8v8L2 12zM11 4h10v3H11zM14 11h7v3h-7zM11 17h10v3H11z"
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

const BASE_COMMANDS = commands.getCommands();
const TOOLBAR_COMMANDS = [
  ...BASE_COMMANDS.slice(0, -2),
  commands.divider,
  indentCommand,
  unindentCommand,
  commands.help,
];

interface Props {
  value: string;
  onChange: (value: string) => void;
  height?: number;
}

export default function QuarterlySummaryMarkdownEditor({
  value,
  onChange,
  height = 350,
}: Props) {
  return (
    <div data-color-mode="light">
      <MDEditor
        value={value}
        onChange={(next) => onChange(next ?? "")}
        height={height}
        commands={TOOLBAR_COMMANDS}
      />
    </div>
  );
}
