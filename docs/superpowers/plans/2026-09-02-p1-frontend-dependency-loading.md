# P1 Frontend Dependency Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 ECharts Core 取代完整入口，并将 Markdown 编辑器变成用户进入编辑态时才加载的异步依赖，显著缩小图表和季度详情同步 chunk，保持所有图表与笔记行为不变。

**Architecture:** 建立唯一的 `EChart` 适配器并只注册仓库实际使用的图表、组件、LabelLayout 与 Canvas renderer；只读 Markdown 改用已有的 `react-markdown`，两个 `@uiw/react-md-editor` 编辑器各自封装到 lazy component。页面容器继续拥有状态、保存、取消和历史加载逻辑，异步组件只负责编辑输入。

**Tech Stack:** React 19、TypeScript 7、Vite 8、ECharts 6、echarts-for-react 3、react-markdown 10、remark-gfm 4、Ant Design 6、Bun。

**Spec:** `docs/superpowers/specs/2026-09-02-p1-simplification-and-read-model-design.md`

## Global Constraints

- 不修改任何图表的 option、序列、颜色、tooltip、legend、dataZoom、markArea 或尺寸语义。
- ECharts 只注册当前扫描确认的 Line、Bar、Pie、Title、Tooltip、Legend、Grid、DataZoom、MarkArea、LabelLayout 与 CanvasRenderer。
- 不通过调高 `chunkSizeWarningLimit`、关闭警告或手写 `manualChunks` 掩盖完整入口。
- `@uiw/react-md-editor` 不得再出现在季度详情的同步模块图中；只读展示不得导入它。
- 季度总结的缩进/反缩进 command、模板文本、保存/取消语义保持原样。
- 持仓笔记 modal 未打开或处于历史模式时，不得触发编辑器 chunk。
- 先取得当前构建基线，再进行 RED/GREEN 构建验证；没有 DOM 测试基础设施时不引入一整套测试框架只为验证第三方 canvas。

---

### Task 1: 固化构建与功能基线

**Files:**
- Inspect: `src/components/charts/*.tsx`
- Inspect: `src/pages/Performance/*Chart.tsx`
- Inspect: `src/pages/Quarterly/ComparisonCharts.tsx`
- Inspect: `src/pages/Quarterly/TrendCharts.tsx`
- Inspect: `src/pages/Quarterly/QuarterlyNotesEditor.tsx`
- Inspect: `src/pages/Quarterly/HoldingNotesEditor.tsx`
- Verify: `vite.config.ts`

**Interfaces:**
- Produces: 当前生产构建的 chunk 名称、原始大小与 gzip 大小。
- Consumes: 现有 Vite 构建输出。

- [ ] **Step 1: 运行生产构建**：

  ```bash
  bun run build
  ```

  保存终端中的 chunk 表；已知基线是完整 ECharts 约 1,141.65 kB/gzip 386.65 kB，`SnapshotDetail` 约 931.47 kB/gzip 325.03 kB。

- [ ] **Step 2: 运行能力扫描**：

  ```bash
  rg -n 'type: "(line|bar|pie)"|title:|tooltip:|legend:|grid:|dataZoom:|markArea:' src/components/charts src/pages/Performance src/pages/Quarterly
  ```

  若发现上述清单之外的新 ECharts 能力，先把对应 module 加入适配器设计，再实施；不得让运行时出现 “component not imported” 警告。

- [ ] **Step 3: 运行同步入口扫描**：

  ```bash
  rg -n 'from "echarts-for-react"|from "@uiw/react-md-editor"' src
  ```

  记录每个命中，作为后续 GREEN 阶段必须收敛的导入集合。

### Task 2: 新增唯一的 ECharts Core 适配器

**Files:**
- Create: `src/components/charts/EChart.tsx`
- Verify: `node_modules/echarts-for-react/lib/index.d.ts`
- Verify: `node_modules/echarts-for-react/lib/core.d.ts`

**Interfaces:**
- Produces: 与 `echarts-for-react` 常用 props 兼容、自动注入裁剪版 ECharts 实例的 `EChart`。
- Consumes: `EChartsReactProps`。

- [ ] **Step 1: 建立 RED 构建检查**：先把一个现有图表临时指向尚不存在的 `./EChart`，运行 `bun run build`，确认 TypeScript 以 module-not-found 失败；随后立即进入下一步完成文件，不提交 RED 状态。

- [ ] **Step 2: 创建适配器**，内容固定为：

  ```tsx
  import ReactEChartsCore from "echarts-for-react/lib/core";
  import type { EChartsReactProps } from "echarts-for-react";
  import * as echarts from "echarts/core";
  import {
    BarChart as EChartsBarChart,
    LineChart as EChartsLineChart,
    PieChart as EChartsPieChart,
  } from "echarts/charts";
  import {
    DataZoomComponent,
    GridComponent,
    LegendComponent,
    MarkAreaComponent,
    TitleComponent,
    TooltipComponent,
  } from "echarts/components";
  import { LabelLayout } from "echarts/features";
  import { CanvasRenderer } from "echarts/renderers";

  echarts.use([
    EChartsBarChart,
    EChartsLineChart,
    EChartsPieChart,
    DataZoomComponent,
    GridComponent,
    LegendComponent,
    MarkAreaComponent,
    LabelLayout,
    TitleComponent,
    TooltipComponent,
    CanvasRenderer,
  ]);

  type Props = Omit<EChartsReactProps, "echarts">;

  export default function EChart(props: Props) {
    return <ReactEChartsCore echarts={echarts} {...props} />;
  }
  ```

- [ ] **Step 3: 运行** `bun run build`，确认 `EChartsReactProps`、core 实例与所有当前 option 均通过类型检查。

### Task 3: 让全部图表使用 Core 适配器

**Files:**
- Modify: `src/components/charts/LineChart.tsx`
- Modify: `src/components/charts/BarChart.tsx`
- Modify: `src/components/charts/PieChart.tsx`
- Modify: `src/pages/Performance/ReturnChart.tsx`
- Modify: `src/pages/Performance/DrawdownChart.tsx`
- Modify: `src/pages/Performance/AttributionChart.tsx`
- Modify: `src/pages/Performance/RankingChart.tsx`
- Modify: `src/pages/Quarterly/ComparisonCharts.tsx`
- Modify: `src/pages/Quarterly/TrendCharts.tsx`

**Interfaces:**
- Removes: 所有 `import ReactECharts from "echarts-for-react"`。
- Preserves: 每个组件的 props 与 JSX 调用形状。

- [ ] **Step 1: 逐文件只替换 import 和组件名**。公共图表使用：

  ```tsx
  import EChart from "./EChart";
  ```

  Performance 与 Quarterly 图表使用：

  ```tsx
  import EChart from "../../components/charts/EChart";
  ```

- [ ] **Step 2: 将每个 `<ReactECharts ... />` 改为 `<EChart ... />`**；`option`、`style`、`opts` 和 event props 原样保留。

- [ ] **Step 3: 运行**：

  ```bash
  rg -n 'from "echarts-for-react"|<ReactECharts' src
  bun run build
  ```

  第一条命令只允许 `EChart.tsx` 中的 type import；不得再有完整默认入口或旧组件名。

- [ ] **Step 4: 对比构建表**，确认原 1.14 MB 完整 ECharts chunk 消失，替代 chunk 只包含注册能力且明显更小。若大小没有明显变化，用 Vite 构建输出和依赖入口定位原因，不调整警告阈值。

### Task 4: 建立轻量 Markdown 预览组件

**Files:**
- Create: `src/components/markdown/MarkdownPreview.tsx`
- Create: `src/components/markdown/markdown-preview.css`
- Modify: `src/pages/Quarterly/QuarterlyNotesEditor.tsx`
- Modify: `src/pages/Quarterly/HoldingNotesEditor.tsx`

**Interfaces:**
- Produces: `{ source: string; className?: string }` 的只读 Markdown renderer。
- Consumes: `react-markdown` 与 `remark-gfm`，不消费 `@uiw/react-md-editor`。

- [ ] **Step 1: 创建轻量 renderer**：

  ```tsx
  import ReactMarkdown from "react-markdown";
  import remarkGfm from "remark-gfm";
  import "./markdown-preview.css";

  interface Props {
    source: string;
    className?: string;
  }

  export default function MarkdownPreview({ source, className = "" }: Props) {
    return (
      <div className={`markdown-preview ${className}`.trim()}>
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{source}</ReactMarkdown>
      </div>
    );
  }
  ```

- [ ] **Step 2: 添加最小样式**，覆盖现有笔记实际会出现的结构：

  ```css
  .markdown-preview { color: var(--color-text); line-height: 1.7; overflow-wrap: anywhere; }
  .markdown-preview > :first-child { margin-top: 0; }
  .markdown-preview > :last-child { margin-bottom: 0; }
  .markdown-preview h1, .markdown-preview h2, .markdown-preview h3 { font-weight: 600; line-height: 1.35; margin: 1em 0 0.5em; }
  .markdown-preview h1 { font-size: 1.5em; }
  .markdown-preview h2 { font-size: 1.3em; }
  .markdown-preview h3 { font-size: 1.15em; }
  .markdown-preview p, .markdown-preview ul, .markdown-preview ol, .markdown-preview blockquote { margin: 0.65em 0; }
  .markdown-preview ul, .markdown-preview ol { padding-left: 1.5em; }
  .markdown-preview ul { list-style: disc; }
  .markdown-preview ol { list-style: decimal; }
  .markdown-preview blockquote { border-left: 3px solid var(--color-border); color: var(--color-text-secondary); padding-left: 0.8em; }
  .markdown-preview code { background: color-mix(in srgb, var(--color-text) 8%, transparent); border-radius: 3px; padding: 0.1em 0.3em; }
  .markdown-preview pre { background: color-mix(in srgb, var(--color-text) 8%, transparent); border-radius: 6px; overflow-x: auto; padding: 0.75em; }
  .markdown-preview table { border-collapse: collapse; width: 100%; }
  .markdown-preview th, .markdown-preview td { border: 1px solid var(--color-border); padding: 0.35em 0.55em; text-align: left; }
  ```

- [ ] **Step 3: 将季度总结预览替换为**：

  ```tsx
  <MarkdownPreview source={notes} />
  ```

- [ ] **Step 4: 将持仓历史中的预览替换为**：

  ```tsx
  <MarkdownPreview source={h.notes} />
  ```

- [ ] **Step 5: 运行** `bun run build`，确认 GFM 表格、列表和类型通过编译，且只读分支不再引用 `MDEditor.Markdown`。

### Task 5: 将季度总结编辑器拆为异步 chunk

**Files:**
- Create: `src/pages/Quarterly/QuarterlySummaryMarkdownEditor.tsx`
- Modify: `src/pages/Quarterly/QuarterlyNotesEditor.tsx`

**Interfaces:**
- Produces: 受控编辑器 `{ value: string; onChange(value: string): void; height?: number }`。
- Preserves: 自定义缩进/反缩进 toolbar 与 `height={350}`。

- [ ] **Step 1: 将以下内容从容器原样移动到新文件**：`INDENT`、`indentCommand`、`unindentCommand`、`BASE_COMMANDS`、`TOOLBAR_COMMANDS` 以及 `@uiw/react-md-editor` 的 imports。

- [ ] **Step 2: 新文件导出受控组件**：

  ```tsx
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
  ```

- [ ] **Step 3: 容器用 lazy import 声明编辑器**：

  ```tsx
  import { lazy, Suspense, useEffect, useState } from "react";
  import { Button, Space, Spin, Typography, message } from "antd";

  const QuarterlySummaryMarkdownEditor = lazy(
    () => import("./QuarterlySummaryMarkdownEditor")
  );
  ```

- [ ] **Step 4: 只在 `editing === true` 的既有分支渲染**：

  ```tsx
  <Suspense fallback={<Spin size="small" />}>
    <QuarterlySummaryMarkdownEditor value={notes} onChange={setNotes} />
  </Suspense>
  ```

  `NOTE_TEMPLATE`、`handleSave`、`handleCancel` 和预览分支继续由容器拥有。

- [ ] **Step 5: 运行** `bun run build`；确认新编辑器文件形成独立异步 chunk，且 `QuarterlyNotesEditor` 本身没有静态 MDEditor import。

### Task 6: 将持仓笔记编辑器拆为异步 chunk

**Files:**
- Create: `src/pages/Quarterly/HoldingMarkdownEditor.tsx`
- Modify: `src/pages/Quarterly/HoldingNotesEditor.tsx`

**Interfaces:**
- Produces: 受控编辑器 `{ value: string; onChange(value: string): void; height?: number }`，固定 `preview="edit"`。
- Preserves: modal、历史请求、模板、保存/关闭和模式切换。

- [ ] **Step 1: 创建异步组件**：

  ```tsx
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
  ```

- [ ] **Step 2: 容器改为 lazy import**：

  ```tsx
  import { lazy, Suspense, useEffect, useState } from "react";

  const HoldingMarkdownEditor = lazy(() => import("./HoldingMarkdownEditor"));
  ```

- [ ] **Step 3: 在现有 `mode === "edit"` 分支中使用**：

  ```tsx
  <Suspense fallback={<Spin size="small" />}>
    <HoldingMarkdownEditor value={notes} onChange={setNotes} />
  </Suspense>
  ```

  该分支位于 `Modal open={open}` 内，确保 modal 未打开或历史模式不会执行动态 import。

- [ ] **Step 4: 运行导入扫描**：

  ```bash
  rg -n 'from "@uiw/react-md-editor"|MDEditor\.Markdown' src/pages/Quarterly src/components
  ```

  结果只能是两个新建的异步编辑器文件中的静态 import；`MDEditor.Markdown` 必须无命中。

### Task 7: 生产构建、冒烟验证与提交

**Files:**
- Verify: all files changed in Tasks 2–6
- Verify: generated `dist/assets/*` output only; do not commit it

**Interfaces:**
- Consumes: Core 图表适配器、轻量预览与异步编辑器。
- Produces: 一个可独立回退的前端性能提交。

- [ ] **Step 1: 运行自动化门禁**：

  ```bash
  bun test
  bun run build
  git diff --check
  ```

- [ ] **Step 2: 对比 Task 1 的构建表**，必须同时满足：
  - 不再出现约 1.14 MB 的完整 ECharts chunk；
  - `SnapshotDetail` 同步 chunk 从约 931 kB 显著下降；
  - `@uiw/react-md-editor` 位于独立异步 chunk；
  - 不修改 `vite.config.ts` 的 warning threshold。

- [ ] **Step 3: 启动应用并冒烟检查图表**：Dashboard 趋势图、Statistics 饼图/柱图、Performance 收益/回撤/归因/排名、Quarterly Comparison 与 Trends 均能绘制，tooltip、缩放和回撤 markArea 正常。

- [ ] **Step 4: 冒烟检查笔记**：季度总结和持仓历史预览能渲染标题、列表、表格；首次进入编辑态显示短暂 loading 后出现编辑器；模板、缩进、反缩进、保存、取消、历史切换均保持原行为。

- [ ] **Step 5: 运行完整质量门禁**：

  ```bash
  bun run check
  git diff --check
  ```

- [ ] **Step 6: 提交**：

  ```bash
  git add src
  git commit -m "perf: load chart and markdown dependencies on demand"
  ```
