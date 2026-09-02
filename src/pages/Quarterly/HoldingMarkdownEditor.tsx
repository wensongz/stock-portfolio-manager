import MDEditor from "@uiw/react-md-editor";

interface Props {
  value: string;
  onChange: (value: string) => void;
  height?: number;
}

export default function HoldingMarkdownEditor({ value, onChange, height = 300 }: Props) {
  return (
    <div data-color-mode="light">
      <MDEditor
        value={value}
        onChange={(next) => onChange(next ?? "")}
        height={height}
        preview="edit"
      />
    </div>
  );
}
