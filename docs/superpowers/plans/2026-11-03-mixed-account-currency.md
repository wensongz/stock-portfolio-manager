# Mixed-account currency fixes

**Goal:** Correctly record and report US/HK stocks and USD/HKD cash in one account, including offline single-currency account statistics.

**Architecture:** Cash market follows its currency, independently of the account's default market. Normalize new cash writes and legacy cash when reading statistics. Account totals use the account market's currency; only cross-currency totals require exchange rates.

**Stack:** React/TypeScript, Tauri/Rust, SQLite.

**Spec:** The three reproduced defects and the user's request in this chat.

## Constraints and decisions

- Preserve stock markets, quantities, cost bases and cash ledger identities.
- Do not invent exchange rates when offline. Single-currency table weights can use native amounts because their ratios and ordering are unchanged.
- Use the current authorized checkout; no branch, commit, push or database migration is needed for these fixes.
- Existing incorrectly classified cash is corrected in the read model without rewriting historical records.

## Tasks

### 1. Cash entry

- [x] Add regression tests for cash deposits/withdrawals with a different account market, and for selecting USD/HKD/CNY in Add Cash.
- [x] Observe incorrect market/currency payloads before changes.
- [x] Share the currency-to-market mapping across the two forms; allow selecting a currency in Add Cash, retaining account defaults as initial values.
- [x] Run the relevant frontend tests.

### 2. Backend cash identity and mixed-account statistics

- [x] Add database-backed tests for cash creation/update, legacy cash classification and independent stock/cash balances within US- and HK-default accounts.
- [x] Observe the cash market regression before changes.
- [x] Normalize cash markets on writes and statistics reads; preserve the cash currency and stock market.
- [x] Verify market totals, account conversion, distributions and independent cash ledgers.

### 3. Offline account statistics

- [x] Add tests proving USD/HKD/CNY single-currency and empty accounts compute without invoking an exchange-rate fetch.
- [x] Verify mixed-currency accounts use cached rates or return the rate-fetch failure without fabricating totals.
- [x] Load holdings before deciding whether rates are required; use optional rates for native-currency account aggregation and table comparison values.
- [x] Run all frontend and Rust tests, frontend build, rustfmt and clippy; review the final diff.

## Review focus

- Legacy cash has the wrong market but the correct currency.
- Cash transaction edits move funds between USD and HKD.
- Same-account stock trades affect only matching-currency cash.
- Empty accounts and accounts without USD holdings work without rates.
- Cached or fetched rates remain mandatory when holdings differ from the account display currency.

## Execution record

- Baseline: clean checkout at b267d79; full frontend and Rust tests started before changes.
- Baseline results: 534 frontend tests passed; 838 Rust tests passed, 13 ignored.
- Cash entry: both frontend regressions observed failing, then passing after fixes.
- Backend: cash-opening, deposit-edit and legacy-classification regressions observed failing, then passing. Integration fixtures verify US/HK account defaults, totals, distributions and cash ledgers before and after stock trades.
- Offline: no-cache native-account regression observed failing with the supplied offline FX future; fixed path never polls that future. USD/HKD/CNY empty accounts and stock/cash weights pass; mixed-currency path uses the real populated ExchangeRateCache.
- Ruling: account alert scopes must include every market in the selected account — required to keep account alerts consistent after legacy cash reclassification. Existing history regression and a new mixed-account fixture both failed before removing the market restriction. Cost if wrong: changed account-alert coverage, now explicitly verified across accounts and currencies.
- Ruling: the AI market-context fixture expected legacy USD cash inside HK totals — update its expectations to HK 2000 and US 2100, preserving historical FX omission checks. Cost if wrong: market context could misstate cash; explicit opposite-market assertions guard this.
- Independent review found two important issues: mixed-account rebalance follow-up still restricted the account to its default market, and required FX legs were not validated. Both were reproduced by failing tests and fixed; the whole Rust suite then passed.
- Ruling: retain validated account identity and the rebalance capability, but omit the default market from account-scoped AI boundaries — otherwise the corrected account preview and follow-up tools disagree. Cost if wrong: expanded research markets; account holdings remain restricted by account ID, and market-scoped boundaries remain enforced.
- Required conversion legs now reject zero, negative, NaN and infinite rates; native-currency aggregation still makes no FX request.
- Ruling: include debit cash balances in the current read model — an actual HK stock purchase funded against USD cash creates negative HKD cash, which must reduce net assets. A new test failed because the debit was omitted, then passed after the query change. Cost if wrong: current net asset values change; the hand-calculated fixture and full regression suite verify the change.
- Review boundaries: historical snapshots are not migrated; missing stock quotes and live provider availability retain existing behavior. These do not require changing the requested current-account currency handling. Negative cash was initially noted outside the review scope, then explicitly added and verified as above.
- Final verification: frontend 536/536 and build passed; Rust 849 passed, 13 existing ignored; clippy all targets/all features with warnings denied, rustfmt and diff whitespace checks passed. No deferred review findings.
