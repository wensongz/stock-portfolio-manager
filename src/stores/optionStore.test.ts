// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const contract = (id: string, accountId: string) => ({
  id,
  option_symbol: `${id} 18SEP26 200 P`,
  underlying: id,
  expiry_date: "2026-09-18",
  strike_price: 200,
  option_type: "P",
  contracts: 1,
  open_price: 2,
  open_amount: 200,
  commission: 1,
  traded_at: "2026-08-01",
  close_price: null,
  close_code: null,
  status: "active",
  account_id: accountId,
});

test("latest option contract request wins when responses resolve out of order", async () => {
  const requests = new Map([
    ["account-a", deferred()],
    ["account-b", deferred()],
  ]);
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command: string, args: { accountId: string }) {
        assert.equal(command, "get_option_contracts");
        return requests.get(args.accountId).promise;
      },
    },
  };

  const { useOptionStore } = await import("./optionStore.ts");
  useOptionStore.setState({ contracts: [], loading: false, error: null });

  const first = useOptionStore.getState().fetchContracts("account-a");
  const second = useOptionStore.getState().fetchContracts("account-b");

  requests.get("account-b").resolve([contract("MSFT", "account-b")]);
  await second;
  requests.get("account-a").resolve([contract("AAPL", "account-a")]);
  await first;

  assert.deepEqual(
    useOptionStore.getState().contracts.map((item) => item.account_id),
    ["account-b"],
  );
  assert.equal(useOptionStore.getState().loading, false);
  assert.equal(useOptionStore.getState().error, null);
});

test("a delayed delete does not clear contracts fetched for a newer account", async () => {
  const deleteRequest = deferred();
  const fetchRequest = deferred();
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command: string, args: { accountId: string }) {
        if (command === "delete_option_records") {
          assert.equal(args.accountId, "account-a");
          return deleteRequest.promise;
        }
        assert.equal(command, "get_option_contracts");
        assert.equal(args.accountId, "account-b");
        return fetchRequest.promise;
      },
    },
  };

  const { useOptionStore } = await import("./optionStore.ts");
  useOptionStore.setState({ contracts: [], loading: false, error: null });

  const deleting = useOptionStore.getState().deleteOptionRecords("account-a");
  const fetching = useOptionStore.getState().fetchContracts("account-b");

  fetchRequest.resolve([contract("MSFT", "account-b")]);
  await fetching;
  deleteRequest.resolve(undefined);
  await deleting;

  assert.deepEqual(
    useOptionStore.getState().contracts.map((item) => item.account_id),
    ["account-b"],
  );
});
