# P3 ECharts Registration Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent the Dashboard pie-chart route from loading bar-, line-, zoom-, and mark-area-specific ECharts implementations.

**Architecture:** Keep one ECharts core instance and React adapter, but split chart registration into a pie entry and a Cartesian entry. Verify the result through the production build graph; remove the change if it does not reduce the Dashboard dependency closure.

**Tech Stack:** React 19, TypeScript 7, Vite 8.1.5, ECharts 6.1, echarts-for-react 3.0.6

**Spec:** `docs/superpowers/specs/2026-09-03-p3-targeted-simplification-design.md`

## Global Constraints

- Preserve every chart option object, renderer selection, route, and visible result.
- Use the same `echarts/core` singleton for both registration entries.
- Do not change `chunkSizeWarningLimit` or add manual vendor chunk configuration.
- Accept the refactor only when the production manifest shows the Dashboard path excludes the Cartesian registration entry.

---

### Task 1: Split Pie and Cartesian Registration

**Files:**
- Create: `src/components/charts/EChartCore.tsx`
- Create: `src/components/charts/PieEChart.tsx`
- Modify: `src/components/charts/EChart.tsx`
- Modify: `src/components/charts/PieChart.tsx`

**Interfaces:**
- Produces: `EChartCore(props)` plus the shared `echarts` singleton.
- Preserves: the existing default `EChart` Cartesian component for all direct consumers.
- Produces: `PieEChart` for `PieChart.tsx` only.

- [ ] **Step 1: Capture a manifest-backed baseline**

Run:

```bash
bunx vite build --manifest
```

Record the `EChart` chunk size (baseline 635.73 kB / 218.35 kB gzip) and inspect `dist/.vite/manifest.json` to confirm the Dashboard dynamic entry currently reaches the shared `EChart` entry used by every chart type.

- [ ] **Step 2: Extract the core renderer**

Create `EChartCore.tsx` with only the shared runtime:

```tsx
import ReactEChartsCore from "echarts-for-react/esm/core.js";
import type { EChartsReactProps } from "echarts-for-react";
import * as echarts from "echarts/core";
import { LabelLayout } from "echarts/features";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([LabelLayout, CanvasRenderer]);

export { echarts };
export type EChartProps = Omit<EChartsReactProps, "echarts">;

export default function EChartCore(props: EChartProps) {
  return <ReactEChartsCore echarts={echarts} {...props} />;
}
```

- [ ] **Step 3: Limit the existing EChart entry to Cartesian charts**

Change `EChart.tsx` to register exactly the dependencies used by bar and line consumers:

```tsx
import { BarChart, LineChart } from "echarts/charts";
import {
  DataZoomComponent, GridComponent, LegendComponent, MarkAreaComponent,
  TitleComponent, TooltipComponent,
} from "echarts/components";
import EChartCore, { echarts, type EChartProps } from "./EChartCore";

echarts.use([
  BarChart, LineChart, DataZoomComponent, GridComponent, LegendComponent,
  MarkAreaComponent, TitleComponent, TooltipComponent,
]);

export default function EChart(props: EChartProps) {
  return <EChartCore {...props} />;
}
```

- [ ] **Step 4: Add the pie-only entry and switch the pie wrapper**

Create `PieEChart.tsx`:

```tsx
import { PieChart } from "echarts/charts";
import { LegendComponent, TitleComponent, TooltipComponent } from "echarts/components";
import EChartCore, { echarts, type EChartProps } from "./EChartCore";

echarts.use([PieChart, LegendComponent, TitleComponent, TooltipComponent]);

export default function PieEChart(props: EChartProps) {
  return <EChartCore {...props} />;
}
```

Update only `src/components/charts/PieChart.tsx` to import `PieEChart` and render it. Leave all Cartesian consumers on `EChart`.

- [ ] **Step 5: Verify TypeScript and the production dependency graph**

Run:

```bash
node --test
bunx tsc
bunx vite build --manifest
```

Expected: 142 or more frontend tests pass; TypeScript and Vite succeed; `dist/.vite/manifest.json` shows Dashboard/PieChart reaches `PieEChart` and the core but not `src/components/charts/EChart.tsx`; the pie path no longer includes `BarChart`, `LineChart`, `DataZoomComponent`, or `MarkAreaComponent` registration code.

- [ ] **Step 6: Compare size and commit only a material split**

Compare the generated pie/core dependency sizes with the 635.73 kB / 218.35 kB shared baseline. If the Dashboard path is not smaller, restore these four files and stop this task. If it is smaller, run:

```bash
git diff --check
git add src/components/charts/EChartCore.tsx src/components/charts/PieEChart.tsx src/components/charts/EChart.tsx src/components/charts/PieChart.tsx
git commit -m "perf: split echarts registrations by chart family"
```
