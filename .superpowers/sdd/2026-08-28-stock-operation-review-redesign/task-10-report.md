# Task 10 report: deterministic stock review assistant

## Status and hash

- Complete.
- Implementation commit: `0f16a78b42d02d393200207d138728c633f305af` (`feat: add deterministic stock review assistant`).
- The pre-existing untracked `node_modules` symlink/directory was preserved and never staged.
- No dependency, schema migration, override/correction AI tool, `openai.yaml`, asset, or unrelated Skill directory was added.

## Skill-writing baseline RED

The required read-only no-Skill pressure evaluation is recorded in `task-10-baseline.md` and ran before this implementation. Its observed failures were:

- no `stock-review` Skill and built-in version 7;
- no `get_stock_review` or `save_stock_review_annotation` tool;
- stock-specific review was governed by generic `trade-review`, including correct/wrong directions and only the latest 20 trades/current holdings;
- option requests either selected generic trade review or selected both generic and option Skills;
- deferred confirmation had no write route and no safeguard against a false save claim;
- high-impact/ambiguity scenarios could not access deterministic report statuses or threshold semantics;
- the no-guidance control retained generic decision-reasonableness instructions and none of the stock-review safeguards.

Baseline counts were Skill service 23/23, AI tools 9/9, and deterministic stock-report boundary 2/2. A configured literal model-output executor was unavailable, so the agreed automated authority is router activation, structured policies, schema/dispatch, persistence boundaries, and value/status preservation.

## Code RED / GREEN evidence

Tests were written before production changes in the existing `ai_tools.rs` and `skill_service.rs` test modules.

Initial RED commands:

```text
cargo test --lib ai_tools::tests -- --nocapture
cargo test --lib skill_service::tests -- --nocapture
```

Both exited 101. The compiler reported the intended missing behavior surface: `StockReviewQuestionCandidate`, structured response-policy enums/functions, `compact_stock_review_payload`, `parse_stock_review_query`, `ToolCtx.stock_review_annotation_confirmation`, the private confirmation constructor, and `export_builtin_skills_to_dir`. The new built-in registration and tool definitions were also absent.

GREEN focused results:

```text
cargo test --lib ai_tools::tests -- --nocapture
14 passed; 0 failed

cargo test --lib skill_service::tests -- --nocapture
28 passed; 0 failed
```

The tests parse/register/activate the real built-in Skill, exercise exact conflict-resolution IDs, consume structured response/question policies, dispatch the real deterministic service, preserve retained JSON values/statuses/issues, and exercise the private annotation confirmation boundary. They do not use Skill-body prose substring checks.

## Tool interfaces and dispatch

### `get_stock_review`

- Required: `start_date`, `end_date`, `base_currency`.
- Optional: `account_id`, `market`, `benchmark_symbol`, `symbol`, `campaign_id`.
- Dates are actionable `YYYY-MM-DD` errors; currency/market/account/benchmark validation reuses Task 9 `validate_query` semantics.
- Dispatch calls `stock_review_service::get_stock_review_for_ai`, which materializes one Task 9 cached input and derives the report plus optional Campaign detail from the same deterministic artifact set.
- The tool never calls the legacy decision-quality service and never recalculates a financial metric.
- Context trimming removes curves, keeps the highest-impact portfolio actions/Campaigns, or filters to the requested symbol/Campaign. It filters only container membership; every retained number, status, fact, and issue remains the exact serialized Task 9 value.
- `available`, `degraded`, `pending`, and `unavailable` remain successful report data. Only malformed parameters, report preparation failure, or a nonexistent/mismatched Campaign are tool errors.
- The result also carries a structured assistant response contract and structured, impact-sorted, maximum-three question candidates.

### `save_stock_review_annotation`

- Required JSON fields: stable `id`, structured `scope`, `annotation_type`, and object-valued `value`.
- `scope.type` uses the existing persistence vocabulary: `period`, `stock`, `campaign`, or `action`; `scope.key` is required and account/symbol are optional.
- No confirmation boolean exists in the schema. The direct executor regression includes a forged `explicitly_confirmed: true` anyway and proves it cannot authorize a write.
- `ToolCtx` receives a private, unconstructable-by-callers capability only from the latest actual user message. The Task 9 service derives it for an explicit save/record request tied to the background/context and rejects deferred/negative phrases.
- Without the capability, execution returns an actionable refusal and the annotation table remains at zero rows. With it, dispatch calls Task 9 `save_ai_confirmed_stock_review_annotation`, preserving the existing private provenance boundary.
- The general Tauri annotation command remains user-provenance-only. No stock-review override/correction tool is registered; corrections remain on the page preview/confirm workflow.

## Activation matrix

Exact automated activation IDs:

| User text | Activated IDs |
| --- | --- |
| `股票操作复盘` | `stock-review` |
| `请做调仓复盘` | `stock-review` |
| `生成股票复盘报告` | `stock-review` |
| `复盘` | `trade-review` |
| `复盘一下我的期权交易` | `options-review` |
| `请做期权复盘` | `options-review` |

The stock Skill has only the narrow triggers `股票操作复盘`, `调仓复盘`, `股票复盘报告`, and `股票Campaign复盘`. Stock routing suppresses broad `trade-review`, `quarterly-report`, and legacy `return-attribution` instructions for the same request; option routing suppresses generic `trade-review`. Generic review remains unchanged when no domain-specific Skill matches.

## Structured response and question policy

The runtime report payload serializes this response order:

1. conclusion;
2. deterministic facts;
3. did well;
4. worth reviewing;
5. cannot infer from data;
6. next-cycle suggestions.

Fact priority is result quality, rebalance value-add, 60/120-day effects, risk structure, then attribution. Evidence classes are fact/status, data inference, user background, and question. Structured flags prohibit recomputation, composite scores, correct/wrong labels, causal claims, quotes/predictions, and concrete trade advice.

Question tests cover both zero and capped nonzero results:

- exactly 20% contribution and exactly 5 percentage points are excluded because the policy is strictly greater-than;
- answered, report-determinable, and prose-completion-only candidates are excluded;
- `-20.0001%`, `5.001pp`, result/risk conflict, and metric-changing ambiguity qualify;
- candidates sort by impact, cap at three, carry exact reason enums, and are all skippable.

## Manual Skill review

The repository-native `src-tauri/src/skills/stock-review.md` was manually checked against the Task 10 brief:

- concise, stock-specific description and triggers; generic `复盘` is absent;
- always reads `get_stock_review` first, with optional symbol/Campaign scope;
- states the six required sections in the required order;
- separates deterministic facts/status, data inference, user background, and pending questions;
- orders result quality/value-add before 60/120, risk, and attribution;
- treats historical manual assessment as display-only user context;
- states all five question predicates, strict thresholds, impact sorting, three-question cap, and skip permission;
- excludes already answered, data-determinable, low-impact, and prose-completion questions;
- calls the annotation writer only after an explicit current-turn save/record request;
- prohibits score, correct/wrong auto-labeling, causality, quote/prediction, concrete buy/sell advice, metric recomputation, and zero-filling unavailable states.

The built-in version is 8. The upgrade regression starts with a version-7 marker, installs `stock-review`, updates built-ins, and preserves a marker-free user Skill.

## Verification

```text
cargo test --lib services::stock_review_persistence::tests -- --nocapture
39 passed; 0 failed

cargo test --lib services::stock_review_service::tests -- --nocapture
50 passed; 0 failed

cargo test --lib
526 passed; 0 failed; 8 ignored

cargo check --lib
passed with no warnings

npm run build
passed; repository's existing large-chunk warning only

rustfmt --edition 2021 --check <four changed Rust files>
passed

git diff --check
passed
```

## Interface rulings and remaining concerns

- Ruling: the AI tool uses snake_case parameters from the Task 10 contract, while existing older tools keep their current camelCase interfaces.
- Ruling: `period` is the report-period annotation scope because it is the established Task 8/9 persistence value; inventing `report` would fail the shared validation boundary.
- Ruling: report and Campaign detail are produced from one materialized artifact set, avoiding two live preparations with potentially different source snapshots.
- Ruling: structured response/question contracts are returned beside the report because literal model execution is unavailable; they make policy behavior executable without testing Markdown wording.
- Ruling: the confirmation capability is derived from the current user turn, not prior conversation text or model arguments. The intentionally narrow classifier may ask a user to restate a vague confirmation, preferring no write over an inferred durable mutation.
- Concern: Task 9's documented live calendar, historical FX/cash, provider total-return, and legacy-ledger limitations remain. The AI path exposes their statuses/issues unchanged and does not repair or hide them.
- Concern: question candidates generated from report data cover contribution, weight, result/risk conflict, and metric-changing ambiguity. Durable reusable context still requires semantic judgment under the Skill instructions; the deterministic selector nevertheless enforces its eligibility, exclusions, sorting, and cap when such a candidate is supplied.
