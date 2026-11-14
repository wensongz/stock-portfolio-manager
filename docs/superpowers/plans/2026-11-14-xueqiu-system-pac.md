# Xueqiu System PAC Fix Implementation Plan

**Goal:** Allow Xueqiu quotes to work with macOS automatic proxy configuration without proxy environment variables.

**Architecture:** Read macOS routing settings with CFNetwork, download the configured PAC script using a direct, cookie-free client, and execute it with the native JavaScript engine on a blocking worker. Preserve explicit environment proxy precedence, NO_PROXY, ordered fallback routes, pooled clients, and a shared cookie jar. Stop official verification redirects and return an actionable warning.

**Tech Stack:** Rust, reqwest 0.13, macOS CFNetwork, core-foundation.

**Spec:** The investigation and confirmed workaround in this conversation. Work in the current checkout; the user approved a local commit after confirming the fix passed testing.

## Constraints and review focus

- Read proxy addresses from system settings; retain normal behavior on other platforms.
- Honor protocol-specific proxy variables before ALL_PROXY and native settings, including NO_PROXY.
- Evaluate PAC as JavaScript using the OS, preserving its URL sanitization and origin-specific DIRECT decisions.
- Bound PAC execution time and release native callbacks/resources on completion or timeout.
- Retain HTTP pooling and cookie storage. Do not read saved credentials during live verification.

## Task 1: Native proxy resolution

- [x] Add regression tests for PAC HTTP proxy selection and DIRECT, environment precedence, NO_PROXY, invalid PAC, and a local HTTP request routed through the selected proxy; observe failures.
- [x] Add a macOS-only native resolver and connect it to the Xueqiu client. Add only the required target-specific dependencies/features.
- [x] Run the proxy tests and verify real Xueqiu JSON with all proxy environment variables removed.

## Task 2: Verification response handling

- [x] Add regression tests for the official verification redirect and its user-facing warning; observe failures.
- [x] Recognize the verification host before JSON parsing without exposing redirect query data.
- [x] Run the regression tests, full Rust suite, rustfmt, clippy, and inspect the final diff.

## Verification record

- Baseline: `cargo test --manifest-path src-tauri/Cargo.toml --lib --offline` — 849 passed, 13 ignored.
- Regression failures observed before fixes: PAC proxy selection, invalid script, explicit proxy environment detection, NO_PROXY matching, verification classification, unavailable primary proxy fallback, and verification redirects to an unreachable destination.
- Full Rust suite: 860 passed, 14 ignored (the additional ignored test is the explicit live-network verification).
- Live verification: removed HTTP_PROXY/HTTPS_PROXY/ALL_PROXY and their lowercase equivalents, then ran `integration_xueqiu_system_pac_without_proxy_environment -- --ignored --nocapture`; received all three requested quotes in 0.67 seconds without saved cookies.
- Infinite-loop script: native evaluation cancelled after the test's 50 ms timeout; the next DIRECT evaluation succeeded. Production download and script execution each have a 3-second timeout, and downloaded scripts are cached for 60 seconds.
- `cargo fmt --all -- --check`, `cargo clippy --offline --all-targets --all-features -- -D warnings`, and `git diff --check` passed.
- Independent review: ordered fallback, early NO_PROXY, and early verification redirect findings addressed; re-review found no remaining important regression. The user confirmed testing passed and requested a local commit.
