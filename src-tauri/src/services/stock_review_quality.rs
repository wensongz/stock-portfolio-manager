#![allow(dead_code)]

use crate::models::stock_review::{
    MetricAvailability, MetricStatus, StockReviewDataQuality, StockReviewIssue,
    StockReviewIssueSeverity,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservationWindowMaturity {
    pub required_market_sessions: u32,
    pub elapsed_market_sessions: u32,
    pub status: MetricStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QualityInput {
    pub market_data_coverage: Option<f64>,
    pub exchange_rate_coverage: Option<f64>,
    pub attribution_residual: Option<f64>,
    pub average_portfolio_nav: Option<f64>,
    pub observation_windows: Vec<ObservationWindowMaturity>,
    pub issues: Vec<StockReviewIssue>,
    pub actual_result_status: MetricStatus,
    pub shadow_value_add_status: MetricStatus,
    pub attribution_status: MetricStatus,
    pub interval_drawdown_only: bool,
}

pub fn classify_coverage_status(coverage_ratio: Option<f64>) -> MetricStatus {
    match coverage_ratio.filter(|ratio| ratio.is_finite() && *ratio >= 0.0 && *ratio <= 1.0) {
        Some(ratio) if ratio >= 0.95 => MetricStatus::Available,
        Some(ratio) if ratio >= 0.80 => MetricStatus::Degraded,
        Some(_) | None => MetricStatus::Unavailable,
    }
}

pub fn classify_residual_status(
    residual: Option<f64>,
    average_portfolio_nav: Option<f64>,
) -> MetricStatus {
    let Some(residual) = residual.filter(|value| value.is_finite()) else {
        return MetricStatus::Unavailable;
    };
    let Some(average_nav) = average_portfolio_nav.filter(|value| value.is_finite() && *value > 0.0)
    else {
        return MetricStatus::Unavailable;
    };
    match residual.abs() / average_nav {
        ratio if ratio <= 0.001 => MetricStatus::Available,
        ratio if ratio <= 0.005 => MetricStatus::Degraded,
        _ => MetricStatus::Unavailable,
    }
}

pub fn merge_metric_statuses(statuses: &[MetricStatus]) -> MetricStatus {
    statuses
        .iter()
        .max_by_key(|status| status_precedence(status))
        .cloned()
        .unwrap_or(MetricStatus::Available)
}

pub fn build_stock_review_quality(input: &QualityInput) -> StockReviewDataQuality {
    let coverage_status = merge_metric_statuses(&[
        classify_coverage_status(input.market_data_coverage),
        classify_coverage_status(input.exchange_rate_coverage),
    ]);
    let residual_status =
        classify_residual_status(input.attribution_residual, input.average_portfolio_nav);
    let maturity_status = merge_metric_statuses(
        &input
            .observation_windows
            .iter()
            .map(|window| {
                if window.elapsed_market_sessions < window.required_market_sessions {
                    merge_metric_statuses(&[window.status.clone(), MetricStatus::Pending])
                } else {
                    window.status.clone()
                }
            })
            .collect::<Vec<_>>(),
    );

    let actual_result_status = input.actual_result_status.clone();
    let mut shadow_value_add_status = merge_metric_statuses(&[
        input.shadow_value_add_status.clone(),
        coverage_status.clone(),
    ]);
    let mut attribution_status = merge_metric_statuses(&[
        input.attribution_status.clone(),
        coverage_status.clone(),
        residual_status,
    ]);
    let forward_effect_status = merge_metric_statuses(&[coverage_status.clone(), maturity_status]);

    let mut issues = input.issues.clone();
    let source_ledger_conflict = issues
        .iter()
        .any(|issue| issue.code == "source_ledger_conflict");
    if source_ledger_conflict {
        shadow_value_add_status = MetricStatus::Unavailable;
        attribution_status = MetricStatus::Unavailable;
        if !issues
            .iter()
            .any(|issue| issue.code == "source_ledger_repair_required")
        {
            issues.push(StockReviewIssue {
                code: "source_ledger_repair_required".to_string(),
                severity: StockReviewIssueSeverity::Error,
                message: "请先修复原始交易记录并重建绩效快照，再重新运行股票复盘。".to_string(),
                affected_symbol: None,
                affected_date: None,
            });
        }
    }
    if input.interval_drawdown_only
        && !issues
            .iter()
            .any(|issue| issue.code == "interval_drawdown_only")
    {
        issues.push(StockReviewIssue {
            code: "interval_drawdown_only".to_string(),
            severity: StockReviewIssueSeverity::Info,
            message: "Maximum drawdown is calculated only from peaks visible inside the selected interval; no pre-window peak is inferred.".to_string(),
            affected_symbol: None,
            affected_date: None,
        });
    }

    let overall_status = merge_metric_statuses(&[
        actual_result_status.clone(),
        shadow_value_add_status.clone(),
        attribution_status.clone(),
        forward_effect_status.clone(),
    ]);

    StockReviewDataQuality {
        availability: availability(overall_status, None),
        actual_result_availability: availability(
            actual_result_status,
            source_ledger_conflict.then(|| {
                "Actual ledger result remains displayable under the recorded ledger path."
                    .to_string()
            }),
        ),
        shadow_value_add_availability: availability(
            shadow_value_add_status,
            source_ledger_conflict.then(|| {
                "Shadow value-add is unavailable until the source transaction record is repaired and snapshots are rebuilt.".to_string()
            }),
        ),
        attribution_availability: availability(
            attribution_status,
            source_ledger_conflict.then(|| {
                "Attribution is unavailable until the source transaction record is repaired and snapshots are rebuilt.".to_string()
            }),
        ),
        forward_effect_availability: availability(
            forward_effect_status.clone(),
            (forward_effect_status == MetricStatus::Pending).then(|| {
                "The 60/120-market-session observation window has not matured.".to_string()
            }),
        ),
        issues,
        market_data_coverage: input.market_data_coverage,
        exchange_rate_coverage: input.exchange_rate_coverage,
        interval_drawdown_only: input.interval_drawdown_only,
    }
}

fn status_precedence(status: &MetricStatus) -> u8 {
    match status {
        MetricStatus::Available => 0,
        MetricStatus::Pending => 1,
        MetricStatus::Degraded => 2,
        MetricStatus::Unavailable => 3,
    }
}

fn availability(status: MetricStatus, note: Option<String>) -> MetricAvailability {
    MetricAvailability { status, note }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available_input() -> QualityInput {
        QualityInput {
            market_data_coverage: Some(0.95),
            exchange_rate_coverage: Some(0.95),
            attribution_residual: Some(1.0),
            average_portfolio_nav: Some(1_000.0),
            observation_windows: vec![],
            issues: vec![],
            actual_result_status: MetricStatus::Available,
            shadow_value_add_status: MetricStatus::Available,
            attribution_status: MetricStatus::Available,
            interval_drawdown_only: false,
        }
    }

    #[test]
    fn coverage_thresholds_are_exact_and_missing_coverage_is_unavailable() {
        // Moving either boundary or treating an absent ratio as zero/complete must fail.
        assert_eq!(
            classify_coverage_status(Some(0.95)),
            MetricStatus::Available
        );
        assert_eq!(
            classify_coverage_status(Some(0.949)),
            MetricStatus::Degraded
        );
        assert_eq!(classify_coverage_status(Some(0.80)), MetricStatus::Degraded);
        assert_eq!(
            classify_coverage_status(Some(0.799)),
            MetricStatus::Unavailable
        );
        assert_eq!(classify_coverage_status(None), MetricStatus::Unavailable);
    }

    #[test]
    fn residual_thresholds_require_a_positive_average_nav() {
        // The ratios are hand-derived: 1/1000=0.1%, 5/1000=0.5%.
        assert_eq!(
            classify_residual_status(Some(1.0), Some(1_000.0)),
            MetricStatus::Available
        );
        assert_eq!(
            classify_residual_status(Some(1.000_1), Some(1_000.0)),
            MetricStatus::Degraded
        );
        assert_eq!(
            classify_residual_status(Some(-5.0), Some(1_000.0)),
            MetricStatus::Degraded
        );
        assert_eq!(
            classify_residual_status(Some(5.000_1), Some(1_000.0)),
            MetricStatus::Unavailable
        );
        assert_eq!(
            classify_residual_status(Some(0.0), Some(0.0)),
            MetricStatus::Unavailable
        );
        assert_eq!(
            classify_residual_status(Some(0.0), Some(f64::NAN)),
            MetricStatus::Unavailable
        );
    }

    #[test]
    fn pending_is_used_only_for_immature_market_session_windows() {
        let mut input = available_input();
        input.attribution_residual = Some(0.0);
        input.observation_windows = vec![
            ObservationWindowMaturity {
                required_market_sessions: 60,
                elapsed_market_sessions: 42,
                status: MetricStatus::Pending,
            },
            ObservationWindowMaturity {
                required_market_sessions: 120,
                elapsed_market_sessions: 42,
                status: MetricStatus::Pending,
            },
        ];

        let pending = build_stock_review_quality(&input);
        assert_eq!(
            pending.forward_effect_availability.status,
            MetricStatus::Pending
        );
        assert_eq!(pending.availability.status, MetricStatus::Pending);

        input.market_data_coverage = Some(0.90);
        let degraded = build_stock_review_quality(&input);
        assert_eq!(
            degraded.forward_effect_availability.status,
            MetricStatus::Degraded
        );
        assert_eq!(degraded.availability.status, MetricStatus::Degraded);
    }

    #[test]
    fn status_merge_uses_unavailable_degraded_pending_available_precedence() {
        assert_eq!(
            merge_metric_statuses(&[MetricStatus::Available, MetricStatus::Pending]),
            MetricStatus::Pending
        );
        assert_eq!(
            merge_metric_statuses(&[MetricStatus::Pending, MetricStatus::Degraded]),
            MetricStatus::Degraded
        );
        assert_eq!(
            merge_metric_statuses(&[MetricStatus::Degraded, MetricStatus::Unavailable]),
            MetricStatus::Unavailable
        );
    }

    #[test]
    fn source_ledger_conflict_preserves_actual_result_but_blocks_replayed_metrics() {
        let mut input = available_input();
        input.attribution_residual = Some(0.0);
        input.interval_drawdown_only = true;
        input.issues.push(StockReviewIssue {
            code: "source_ledger_conflict".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "Duplicate correction conflicts with the source ledger.".to_string(),
            affected_symbol: Some("A".to_string()),
            affected_date: None,
        });

        let quality = build_stock_review_quality(&input);

        assert_eq!(
            quality.actual_result_availability.status,
            MetricStatus::Available
        );
        assert_eq!(
            quality.shadow_value_add_availability.status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            quality.attribution_availability.status,
            MetricStatus::Unavailable
        );
        assert!(quality.interval_drawdown_only);
        assert!(quality.issues.iter().any(|issue| {
            issue.code == "source_ledger_repair_required"
                && issue.message.contains("修复原始交易记录")
        }));
        assert!(quality
            .issues
            .iter()
            .any(|issue| issue.code == "interval_drawdown_only"));
    }
}
