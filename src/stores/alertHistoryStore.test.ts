// @ts-nocheck
import assert from "node:assert/strict";
import test from "node:test";
import { createAlertHistoryStore } from "./alertHistoryStore.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function historyPage(title, page = 1, pageSize = 20) {
  return {
    items: [{
      id: `message-${title}`,
      kind: "PORTFOLIO",
      title,
      message: `${title} summary`,
      scopeName: "美股组合",
      accountName: "长期投资账户",
      triggeredAt: "2026-09-19T08:09:10Z",
      details: [{ label: "当前仓位", value: "72.5%" }],
    }],
    total: 1,
    page,
    pageSize,
  };
}

test("history requests use the camelCase query contract and default pagination", async () => {
  const calls = [];
  const store = createAlertHistoryStore(async (command, args) => {
    calls.push({ command, args });
    return historyPage("首次触发");
  });

  await store.getState().fetchHistory();

  assert.deepEqual(calls, [{
    command: "get_alert_history",
    args: { query: { page: 1, pageSize: 20 } },
  }]);
  assert.equal(store.getState().items[0].accountName, "长期投资账户");
});

test("an older response cannot overwrite a newer filtered result", async () => {
  const first = deferred();
  const second = deferred();
  let callCount = 0;
  const store = createAlertHistoryStore(() => {
    callCount += 1;
    return callCount === 1 ? first.promise : second.promise;
  });

  const staleRequest = store.getState().fetchHistory();
  const freshRequest = store.getState().setKind("PRICE");
  second.resolve({ ...historyPage("价格提醒"), items: [{ ...historyPage("价格提醒").items[0], kind: "PRICE" }] });
  await freshRequest;
  first.resolve(historyPage("过期组合提醒"));
  await staleRequest;

  assert.equal(store.getState().kind, "PRICE");
  assert.equal(store.getState().items[0].title, "价格提醒");
  assert.equal(store.getState().loading, false);
});

test("filters reset to the first page and pagination preserves active criteria", async () => {
  const queries = [];
  const store = createAlertHistoryStore(async (_command, { query }) => {
    queries.push(query);
    return { ...historyPage("结果", query.page, query.pageSize), total: query.page * query.pageSize };
  });

  await store.getState().setPagination(3, 50);
  await store.getState().setSearch("苹果");
  await store.getState().setKind("PORTFOLIO");

  assert.deepEqual(queries, [
    { page: 3, pageSize: 50 },
    { search: "苹果", page: 1, pageSize: 50 },
    { kind: "PORTFOLIO", search: "苹果", page: 1, pageSize: 50 },
  ]);
});

test("retry clears the previous error after a successful request", async () => {
  let attempts = 0;
  const store = createAlertHistoryStore(async () => {
    attempts += 1;
    if (attempts === 1) throw new Error("暂时无法读取历史消息");
    return historyPage("恢复后的结果");
  });

  await store.getState().fetchHistory();
  assert.match(store.getState().error ?? "", /暂时无法读取/);

  await store.getState().fetchHistory();
  assert.equal(store.getState().error, null);
  assert.equal(store.getState().items[0].title, "恢复后的结果");
});

test("deleting the last message on a filtered last page returns to the remaining page", async () => {
  const queries = [];
  let deleted = false;
  const store = createAlertHistoryStore(async (command, args) => {
    if (command === "delete_alert_history") {
      assert.deepEqual(args, { id: "message-待删除" });
      deleted = true;
      return true;
    }
    queries.push(args.query);
    if (args.query.page === 2) {
      return {
        ...historyPage("待删除", 2), total: deleted ? 20 : 21,
        items: deleted ? [] : historyPage("待删除").items,
      };
    }
    return { ...historyPage("保留消息"), total: 20 };
  });
  await store.getState().setKind("PORTFOLIO");
  await store.getState().setSearch("账户");
  await store.getState().setPagination(2, 20);

  assert.equal(await store.getState().deleteMessage("message-待删除"), true);

  assert.equal(store.getState().page, 1);
  assert.equal(store.getState().total, 20);
  assert.deepEqual(store.getState().items.map((item) => item.title), ["保留消息"]);
  assert.deepEqual(queries.slice(-2), [
    { kind: "PORTFOLIO", search: "账户", page: 2, pageSize: 20 },
    { kind: "PORTFOLIO", search: "账户", page: 1, pageSize: 20 },
  ]);
  assert.equal(store.getState().deletingId, null);
});

test("a failed deletion preserves the message and permits a retry", async () => {
  let attempts = 0;
  const store = createAlertHistoryStore(async (command) => {
    if (command === "delete_alert_history") {
      attempts += 1;
      if (attempts === 1) throw new Error("数据库暂时不可用");
      return true;
    }
    return attempts > 1
      ? { ...historyPage(""), items: [], total: 0 }
      : historyPage("保留消息");
  });
  await store.getState().fetchHistory();
  await assert.rejects(store.getState().deleteMessage("message-保留消息"), /数据库暂时不可用/);
  assert.equal(store.getState().items[0].title, "保留消息");
  assert.equal(store.getState().total, 1);
  assert.equal(store.getState().deletingId, null);
  assert.equal(await store.getState().deleteMessage("message-保留消息"), true);
  assert.deepEqual(store.getState().items, []);
  assert.equal(store.getState().page, 1);
});

test("in-flight stale reads cannot bring back a deleted message, even if refresh fails", async () => {
  const stale = deferred();
  const deletion = deferred();
  let reads = 0;
  let deletions = 0;
  const store = createAlertHistoryStore(async (command) => {
    if (command === "delete_alert_history") {
      deletions += 1;
      return deletion.promise;
    }
    reads += 1;
    if (reads === 1) return historyPage("待删除");
    if (reads === 2) return stale.promise;
    throw new Error("刷新失败");
  });
  await store.getState().fetchHistory();
  const oldRequest = store.getState().fetchHistory();
  const removing = store.getState().deleteMessage("message-待删除");
  assert.equal(store.getState().deletingId, "message-待删除");
  assert.equal(await store.getState().deleteMessage("message-待删除"), false);
  assert.equal(deletions, 1);
  deletion.resolve(true);
  assert.equal(await removing, true);
  stale.resolve(historyPage("待删除"));
  await oldRequest;
  assert.deepEqual(store.getState().items, []);
  assert.equal(store.getState().total, 0);
  assert.match(store.getState().error, /刷新失败/);
  assert.equal(store.getState().loading, false);
});

test("deletion refreshes the latest filters if they change while deletion is pending", async () => {
  const deletion = deferred();
  const queries = [];
  const store = createAlertHistoryStore(async (command, args) => {
    if (command === "delete_alert_history") return deletion.promise;
    queries.push(args.query);
    return historyPage(args.query.kind === "PRICE" ? "价格消息" : "组合消息");
  });
  await store.getState().fetchHistory();
  const removing = store.getState().deleteMessage("message-组合消息");
  await store.getState().setKind("PRICE");
  deletion.resolve(false); // A message already removed elsewhere is still a successful outcome.
  assert.equal(await removing, true);
  assert.equal(store.getState().kind, "PRICE");
  assert.equal(store.getState().items[0].title, "价格消息");
  assert.deepEqual(queries.at(-1), { kind: "PRICE", page: 1, pageSize: 20 });
});

test("retry after a failed post-delete refresh still recovers from an empty last page", async () => {
  let deleted = false;
  let refreshFailed = false;
  const store = createAlertHistoryStore(async (command, { query }) => {
    if (command === "delete_alert_history") {
      deleted = true;
      return true;
    }
    if (deleted && !refreshFailed) {
      refreshFailed = true;
      throw new Error("刷新失败");
    }
    return {
      ...historyPage("仍保留的消息", query.page),
      items: query.page > 1
        ? deleted ? [] : historyPage("待删除").items
        : historyPage("仍保留的消息").items,
      total: deleted ? 20 : 21,
    };
  });
  await store.getState().setPagination(2, 20);
  await store.getState().deleteMessage("message-待删除");
  assert.match(store.getState().error, /刷新失败/);

  await store.getState().fetchHistory();

  assert.equal(store.getState().page, 1);
  assert.equal(store.getState().total, 20);
  assert.equal(store.getState().items[0].title, "仍保留的消息");
  assert.equal(store.getState().error, null);
});
