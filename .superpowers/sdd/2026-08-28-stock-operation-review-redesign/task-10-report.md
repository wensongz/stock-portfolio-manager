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

---

## Fix round 1: boundary hardening (2026-08-29)

Implementation commit: `180c9a91c152884965caa695bcf632a1795742ad` (`fix: harden stock review assistant boundaries`). This section supersedes the original current-turn text-classifier ruling above.

### Review RED evidence

The new routing matrix was added before the router change and run with:

```text
cargo test --lib services::skill_service::tests::review_routing_requires_intent_and_keeps_stock_and_option_domains_disjoint -- --exact
```

It exited 101: `股票 Campaign 复盘` activated `trade-review` instead of `stock-review`.

The tool boundary, exact-draft confirmation, scoped projection, question-selection, cap, and validation regressions were then added before production changes and run with:

```text
cargo test --lib services::ai_tools::tests -- --nocapture
```

It exited 101 because the trusted untrusted-turn context and exact-draft test capability did not exist. The pre-fix code also still exposed `confirmed_ai_annotation_capability_for_user_turn`, which inferred write authority from natural-language substrings. These were the intended REDs; no Skill-body substring assertion was used.

### Confirmation boundary

- Natural-language confirmation inference was deleted. Both OpenAI-style and Anthropic chat paths now construct an untrusted-turn tool context that always starts without write capability, including quoted, hypothetical, interrogative, negated, and model-supplied confirmation text.
- The private capability is bound to normalized ID, scope, annotation type, source, canonical JSON value, and its stable value hash. JSON object key order is canonicalized, while a different value or scope does not match.
- The capability uses an atomic one-shot state. A mismatched draft has zero side effect and does not consume the approval; the first exact write consumes it; an exact replay or any later payload fails with `confirmation_required` and leaves the row count unchanged.
- Tool JSON has no confirmation field. Unknown root/scope fields, a forged `explicitly_confirmed`, and non-string/blank/null optional strings are rejected before persistence.
- There is intentionally no production capability constructor until a trusted host/UI approval event is implemented. The safe production behavior is therefore read-only plus `confirmation_required`; ordinary chat text cannot cross the Task 9 private AI-confirmed persistence boundary.

### Routing matrix

Exact forward activation IDs after the fix:

| User text | Activated IDs |
| --- | --- |
| `股票 Campaign 复盘` | `stock-review` |
| `复盘一下我的期权交易` | `options-review` |
| `请做期权历史表现复盘` | `options-review` |
| `导入一笔期权交易` | `trade-review` only; never `options-review` |
| `Campaign复盘` | `trade-review` |
| `复盘` | `trade-review` |
| `一起复盘股票调仓和期权交易` | `options-review`, `stock-review` |

The options Skill no longer has bare `期权交易` or `Campaign复盘` triggers. Runtime resolution requires review intent plus an explicit stock/option domain for the two specialized built-ins, preserves generic fallback, and supports a genuinely mixed request.

### Campaign/symbol projection and bounded context

- Campaign scope filters report actions and all attribution arrays by the Campaign's exact `action_ids`, not by symbol. A two-Campaign/same-symbol fixture proves that the other cycle's 99-unit contribution and annotations do not leak.
- Campaign annotations are limited to the exact Campaign, its exact actions, and account/symbol-free period context. Campaign detail annotations use the same filter.
- Symbol matching reuses the shared trimmed, case-normalized stock-symbol identity helper.
- Question eligibility, semantic deduplication, impact ordering, and maximum-three selection run on the complete scoped report before display caps. Repeated issue rows collapse by code + normalized scope + date. Existing action, Campaign, issue, and global structured context suppresses already-answered questions.
- The 6pp regression places a qualifying action beyond the normal top 12 and proves both its question and referenced action survive the cap.
- Every variable report/detail array is recursively bounded. High-value paths use explicit 12/20/40 limits; nested arrays use a deterministic default limit. `context_limits` records `limit`, `total`, `returned`, and `omitted` for every path, including deliberately removed curves. Ranked collections use stable tie-breaking and selected-question references take priority without altering any retained value/status.

### Strict input contract

- Runtime validation now mirrors `additionalProperties: false` rather than trusting provider schema enforcement.
- `get_stock_review` rejects unknown fields and present optional values that are null, blank, or non-string, so invalid scope never silently broadens.
- Annotation root and scope objects reject unknown fields. `value` must be an object. `period` rejects a symbol; `stock` requires a normalized matching symbol/key; action/Campaign/period scopes retain only their documented optional account/symbol fields.
- All invalid-shape regressions assert a zero annotation row count.

### Manual Skill review

The repository-native Skills remain concise and behavior-first. `stock-review.md` still requires deterministic read-first behavior, the six-section order, fact/status/background/inference separation, result/value-add priority, strict question predicates, and all no-recompute/no-score/no-causality/no-quote/no-prediction/no-concrete-trade-advice constraints. Its write section now requires trusted exact-draft host confirmation, treats chat text/tool arguments as non-authority, and forbids retrying or claiming success after `confirmation_required`. `options-review.md` retains its deterministic options-review instructions with only narrow review triggers.

### GREEN verification

```text
cargo test --lib services::ai_tools::tests -- --nocapture
20 passed; 0 failed

cargo test --lib services::skill_service::tests -- --nocapture
29 passed; 0 failed

cargo test --lib services::stock_review_persistence::tests -- --nocapture
39 passed; 0 failed

cargo test --lib services::stock_review_service::tests -- --nocapture
50 passed; 0 failed

cargo test --lib
533 passed; 0 failed; 8 ignored

cargo check --lib
passed with no warnings

npm run build
passed; repository's existing large-chunk warning only

rustfmt --edition 2021 --check <four changed Rust files>
passed

git diff --check
passed
```

### Interface rulings and remaining concern

- Ruling: host confirmation is a separate capability artifact, not a boolean/string tool argument and not a linguistic classification. Until a trusted approval UI is wired, annotation writes remain safely unavailable from production chat.
- Ruling: question selection occurs after semantic scope filtering but before any size cap; this keeps symbol/Campaign answers relevant without losing lower-ranked qualifying rows.
- Ruling: collection omission metadata is part of successful result data, alongside Task 9 metric availability/statuses; it is never a tool error.
- Remaining concern: the trusted host/UI approval event is deliberately not fabricated in this task. A future UI integration must construct the private exact-draft capability inside the trusted boundary rather than reintroducing text inference or a caller-supplied token.

---

## Fix round 2: migration, routing, and issue-order finalization (2026-08-29)

Implementation commit: `8feb8ed07d354d0a6715ad23863148d3c0086526` (`fix: finalize stock review skill routing`). This section supersedes the earlier statement that the current built-in version is 8; the hardened built-in version is 9.

### RED evidence

The v8 migration, on-disk routing, stock negative-pressure matrix, and deterministic issue-cap tests were written before production changes.

```text
cargo test --lib services::skill_service::tests -- --nocapture
```

Exited 101 with three intended failures:

- a v8-marked `options-review.md` remained the old broad-trigger file instead of upgrading;
- the version-7 migration still wrote marker 8 instead of 9;
- `这只股票历史表现怎么样` incorrectly activated `stock-review`.

```text
cargo test --lib services::ai_tools::tests::issue_caps_are_byte_stable_for_equivalent_permuted_inputs -- --exact --nocapture
```

Exited 101 because reversing an equivalent 32-issue array changed both selected ordering and which unselected issues survived the 20-item cap.

### Version-9 migration and disk-loaded routing

- `BUILTIN_SKILLS_VERSION` is 9 because `stock-review` and `options-review` changed after the v8 release.
- The migration fixture starts with old v8-marked copies of both review Skills, runs the real exporter, and compares both resulting files byte-for-byte with the current embedded built-ins.
- Both markers advance to 9. A marker-free custom Skill remains byte-identical and receives no built-in marker.
- The activation assertions parse `trade-review.md`, `options-review.md`, and `stock-review.md` from the migrated temporary directory, then route those parsed on-disk Skills. They do not substitute embedded constants for the routed content.

### Narrow stock semantics and pressure matrix

Stock auto-activation now requires an explicit review intent plus one of:

- stock-operation semantics;
- rebalance semantics;
- stock Campaign-review semantics;
- the explicit stock-review-report phrase.

Option-specific historical aliases remain available only when an option domain is present. Generic `历史表现` and `历史Campaign` no longer create a domain-neutral review intent.

| User text | Stock-review result |
| --- | --- |
| `这只股票历史表现怎么样` | inactive |
| `查询这只股票的风险` | inactive |
| `导入股票交易记录` | inactive |
| `查询股票行情` | inactive |
| `股票操作复盘` | `stock-review` only |
| `复盘股票调仓` | `stock-review` only |
| `股票 Campaign 复盘` | `stock-review` only |
| `一起复盘股票调仓和期权交易` | `options-review`, `stock-review` |

The separate option cases continue to route option history/review aliases without activating stock review.

### Deterministic issue cap

Report and Campaign-detail issues now sort by:

1. selected-question priority;
2. trimmed issue code;
3. shared trim/case-normalized symbol identity;
4. affected date;
5. recursively canonicalized `value`;
6. recursively canonicalized `details`;
7. recursively canonicalized remaining fields.

Length-prefixed key components avoid concatenation ambiguity. The cap layer is self-sufficient even if an upstream producer supplies a HashSet-derived permutation. The regression reverses 32 equivalent issues and proves byte-identical retained report issues, Campaign-detail issues, and omission metadata.

### GREEN verification

```text
cargo test --lib services::ai_tools::tests -- --nocapture
21 passed; 0 failed

cargo test --lib services::skill_service::tests -- --nocapture
31 passed; 0 failed

cargo test --lib services::stock_review_persistence::tests -- --nocapture
39 passed; 0 failed

cargo test --lib services::stock_review_service::tests -- --nocapture
50 passed; 0 failed

cargo test --lib
536 passed; 0 failed; 8 ignored

cargo check --lib
passed with no warnings

npm run build
passed; repository's existing large-chunk warning only

rustfmt --edition 2021 --check <two changed Rust files>
passed

git diff --check
passed
```

No new interface concern was introduced. The prior trusted-host confirmation integration concern remains unchanged.
