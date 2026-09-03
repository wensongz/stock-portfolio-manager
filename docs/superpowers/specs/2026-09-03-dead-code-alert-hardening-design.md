# Dead Code and Alert Hardening Design

## Problem

Several UI-to-Tauri call chains have no consumers, while alert checking ignores database update failures and the AI tool converts alert-check errors into an empty success result.

## Design

Remove the unused LineChart component and the complete unused chains for expired option statistics, frontend-triggered alert checks, and quarterly notes summaries. This includes frontend state/types, Tauri command registration, thin commands, and Rust models or service functions that have no remaining consumer.

Keep `alert_service::check_alerts` because the AI tool uses it. Load alerts and update all triggered statuses inside one transaction. Any update failure rolls back the batch and is returned. The AI tool maps that error to an explicit tool error rather than an empty triggered list.

## Verification

Reference searches must find no dead command names or stale types. Alert tests will force an update failure and verify rollback, and an AI tool test will verify that the service error is visible to the caller.
