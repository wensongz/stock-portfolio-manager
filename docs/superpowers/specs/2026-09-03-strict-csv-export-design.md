# Strict CSV Export Decoding Design

## Problem

CSV export row mapping silently replaces SQLite decoding failures with empty strings or zero values. Corrupted or incompatible database values can therefore be exported as plausible but false financial data.

## Design

Introduce typed export row structures whose `from_row` functions return `rusqlite::Result`. Required database values use strict `row.get`; genuinely nullable joined labels and notes use `Option<T>` and are deliberately rendered as blank CSV cells. Database lock failures are also returned instead of panicking.

CSV serialization behavior, column order, filters, and valid-data output remain unchanged.

## Verification

Tests will place text in numeric SQLite columns and assert that holdings and transaction exports return errors. Existing valid export tests will continue to prove stable CSV output.
