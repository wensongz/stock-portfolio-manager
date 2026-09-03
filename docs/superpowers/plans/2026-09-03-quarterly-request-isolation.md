# Quarterly Request Isolation Implementation Plan

**Goal:** Prevent unrelated loading state and stale quarterly responses from interfering with the active view.

**Architecture:** Each store slice has independent request status and generation guards; detail plus transactions form one snapshot-scoped bundle.

**Tech Stack:** TypeScript, Zustand, React, Node tests.

---

### Task 1: Capture stale-response failures

**Files:**
- Create: `src/stores/quarterlyStore.test.ts`
- Modify: `src/stores/quarterlyStore.ts`

1. Add failing deferred-promise tests for A-to-B detail switches, stale errors, comparison pair switches, and independent concurrent loading flags.
2. Run `bun test src/stores/quarterlyStore.test.ts` and confirm failure.

### Task 2: Split state and add guards

**Files:**
- Modify: `src/stores/quarterlyStore.ts`

1. Add independent list/detail/comparison/trends/mutation statuses.
2. Add generation and request-key checks for detail, transactions, comparison, and trends.
3. Clear detail and transactions together when selection changes.
4. Run the focused store tests.

### Task 3: Bind pages to their slice

**Files:**
- Modify: `src/pages/Quarterly/index.tsx`
- Modify: `src/pages/Quarterly/SnapshotDetail.tsx`
- Modify: `src/pages/Quarterly/QuarterComparison.tsx`
- Modify: `src/pages/Quarterly/TrendsPage.tsx`

1. Replace shared loading/error consumption with the matching slice state.
2. Run `bun run test` and `bun run build`.
3. Commit as `fix: isolate quarterly request state`.
