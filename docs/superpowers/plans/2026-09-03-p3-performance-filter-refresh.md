# P3 Performance Filter Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove zero-delay timer scheduling from performance filters while guaranteeing the refresh uses the newly selected market or account.

**Architecture:** Make the Zustand filter actions own the complete state transition: synchronously update mutually exclusive filters, then invoke the existing request-safe `fetchAll` action and return its Promise. The page becomes a thin event adapter and no longer coordinates store timing.

**Tech Stack:** React 19, TypeScript 7, Zustand 5, Node test runner

**Spec:** `docs/superpowers/specs/2026-09-03-p3-targeted-simplification-design.md`

## Global Constraints

- Preserve market/account mutual exclusion and latest-request-wins behavior.
- Preserve the `get_performance_report` request shape and existing successful data during refresh failures.
- Do not use timers, microtask scheduling, or React effects to trigger filter refreshes.

---

### Task 1: Make Filter Selection and Refresh One Store Action

**Files:**
- Modify: `src/stores/performanceStore.test.ts`
- Modify: `src/stores/performanceStore.ts`
- Modify: `src/pages/Performance/index.tsx`

**Interfaces:**
- Changes: `setMarket(market: string | null) => Promise<void>`.
- Changes: `setAccountId(accountId: string | null) => Promise<void>`.
- Preserves: all backend commands and report state fields.

- [ ] **Step 1: Write the failing immediate-refresh test**

Add this test using the existing literal `report()` fixture:

```typescript
test("market and account selections refresh with the newly written filter", async () => {
  const reportCalls = [];
  const invoke = async (command, args) => {
    if (command === "backfill_snapshots") return 0;
    assert.equal(command, "get_performance_report");
    reportCalls.push(args);
    return report(`result-${reportCalls.length}`);
  };
  const store = createPerformanceStore(invoke);

  await store.getState().setMarket("HK");
  assert.equal(reportCalls[0].market, "HK");
  assert.equal(reportCalls[0].accountId, undefined);

  await store.getState().setAccountId("account-1");
  assert.equal(reportCalls[1].accountId, "account-1");
  assert.equal(reportCalls[1].market, undefined);
});
```

- [ ] **Step 2: Run the test to verify RED**

Run:

```bash
node --test src/stores/performanceStore.test.ts
```

Expected: FAIL because the existing setters return `void` and do not call `get_performance_report`.

- [ ] **Step 3: Implement the atomic store actions**

Change the interface and implementations to:

```typescript
setMarket: (market: string | null) => Promise<void>;
setAccountId: (accountId: string | null) => Promise<void>;

setMarket: async (market) => {
  set({ selectedMarket: market, selectedAccountId: null });
  await get().fetchAll();
},

setAccountId: async (accountId) => {
  set({ selectedAccountId: accountId, selectedMarket: null });
  await get().fetchAll();
},
```

- [ ] **Step 4: Remove page-level timer scheduling**

Replace both handlers in `src/pages/Performance/index.tsx`:

```tsx
const handleMarketChange = (value: string | undefined) => {
  void setMarket(value ?? null);
};

const handleAccountChange = (value: string | undefined) => {
  void setAccountId(value ?? null);
};
```

- [ ] **Step 5: Verify GREEN and regression behavior**

Run:

```bash
node --test src/stores/performanceStore.test.ts
bun run build
```

Expected: the new test and all existing stale-response/failure-preservation tests pass; TypeScript and Vite build succeed.

- [ ] **Step 6: Commit**

Run:

```bash
git diff --check
git add src/stores/performanceStore.test.ts src/stores/performanceStore.ts src/pages/Performance/index.tsx
git commit -m "refactor: refresh performance filters without timers"
```

### Task 2: Run the Complete P3 Gate

**Files:**
- Verify: every file changed by the three P3 plans.

**Interfaces:**
- Consumes: all prior P3 commits.
- Produces: fresh evidence that public behavior, compilation, formatting, and linting remain intact.

- [ ] **Step 1: Run the full repository check**

Run:

```bash
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target bun run check
git diff --check
git status --short
```

Expected: all frontend and Rust tests pass, production build succeeds, Rust formatting and strict Clippy are clean, and only the approved P3 plan/implementation files are present.
