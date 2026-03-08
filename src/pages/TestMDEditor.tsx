import { useState } from "react";
import MDEditor from "@uiw/react-md-editor";

export default function TestMDEditor() {
  const [value, setValue] = useState("# Hello World\n\nThis is a test");
  return (
    <div style={{ padding: 20 }}>
      <h2>MDEditor Test</h2>
      <div data-color-mode="light">
        <MDEditor value={value} onChange={(v) => setValue(v ?? "")} height={350} />
      </div>
      <hr />
      <h3>Preview:</h3>
      <div data-color-mode="light">
        <MDEditor.Markdown source={value} />
      </div>
    </div>
  );
}
