use renderer_demo::display::DisplayMode;

use crate::{orbit_camera::ProjectionMode, review::ReviewStatus, scene::SceneMetrics};

pub(crate) const MAX_STATUS_COLUMNS: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamStatus {
    Loading,
    Settling,
    Steady,
    LoadsPaused,
}

impl StreamStatus {
    pub(crate) const fn from_facts(
        scene: SceneMetrics,
        loads_paused: bool,
        planned_work: bool,
        transition_active: bool,
    ) -> Self {
        if loads_paused {
            return Self::LoadsPaused;
        }
        let pending_work = planned_work
            || transition_active
            || scene.queued_batches > 0
            || scene.requested_nodes > 0
            || scene.staged_points > 0;
        if scene.resident_batches == 0 && (scene.logical_points > 0 || pending_work) {
            Self::Loading
        } else if pending_work {
            Self::Settling
        } else {
            Self::Steady
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Loading => "LOADING",
            Self::Settling => "SETTLING",
            Self::Steady => "STEADY",
            Self::LoadsPaused => "LOADS PAUSED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StatusSnapshot {
    pub(crate) display: DisplayMode,
    pub(crate) projection: ProjectionMode,
    pub(crate) stream: StreamStatus,
    pub(crate) scene: SceneMetrics,
    pub(crate) drawn_points: u64,
    pub(crate) selected: Option<ReviewStatus>,
    pub(crate) resident_highlights: u64,
    pub(crate) orientation: &'static str,
    pub(crate) scale_world_units: f64,
    pub(crate) cursor_world: Option<[f64; 3]>,
}

impl StatusSnapshot {
    pub(crate) fn lines(self) -> Vec<String> {
        let (selection_state, selected_points) = selection(self.selected);
        let mut lines = vec![
            format!("PUNCTRA {} | VIEW PRE-V0.13", env!("CARGO_PKG_VERSION")),
            format!(
                "{} | {} | {}",
                self.display,
                self.projection,
                self.stream.label()
            ),
            coverage_line(self.scene),
            "QUERY COMPLETION NOT IMPLIED".to_owned(),
            format!(
                "SOURCE {} | DRAWN {} | RESIDENT {}",
                compact(self.scene.logical_points),
                compact(self.drawn_points),
                compact(self.scene.resident_points)
            ),
            format!(
                "SELECTED {} | RESIDENT LOCATORS {}",
                compact(selected_points),
                compact(self.resident_highlights)
            ),
            format!("SELECTION {selection_state} | X CLEAR"),
            format!(
                "NORTH {} | SCALE 100PX = {}",
                self.orientation,
                compact_decimal(self.scale_world_units)
            ),
            cursor_line(self.cursor_world),
            palette_line(self.display).to_owned(),
            "H LOCATORS | SPACE LOADS | P PROJECTION".to_owned(),
        ];
        for line in &mut lines {
            line.make_ascii_uppercase();
            if line.len() > MAX_STATUS_COLUMNS {
                line.truncate(MAX_STATUS_COLUMNS);
            }
        }
        lines
    }
}

fn coverage_line(scene: SceneMetrics) -> String {
    if scene.authored_resident_batches > 0 {
        format!(
            "COVERAGE SAMPLED {} COMPLETE {} AUTHORED {}",
            scene.sampled_resident_batches,
            scene.complete_resident_batches,
            scene.authored_resident_batches
        )
    } else {
        format!(
            "COVERAGE SAMPLED {} / COMPLETE {}",
            scene.sampled_resident_batches, scene.complete_resident_batches
        )
    }
}

fn selection(status: Option<ReviewStatus>) -> (&'static str, u64) {
    match status {
        None => ("DISABLED", 0),
        Some(ReviewStatus::Selected { points, .. }) => ("EXACT", points),
        Some(ReviewStatus::SelectionStale { points, .. }) => ("STALE - RERUN OR CLEAR", points),
        Some(ReviewStatus::ProvisionalPick | ReviewStatus::ConfirmingPick) => ("CONFIRMING", 0),
        Some(ReviewStatus::SelectingScreen) => ("SELECTING", 0),
        Some(ReviewStatus::Failed | ReviewStatus::Indeterminate) => ("FAILED - X CLEAR", 0),
        Some(_) => ("READY", 0),
    }
}

fn cursor_line(cursor: Option<[f64; 3]>) -> String {
    cursor.map_or_else(
        || "CURSOR WORLD OUTSIDE VIEW".to_owned(),
        |[x, y, z]| {
            format!(
                "CURSOR X {} Y {} Z {}",
                compact_decimal(x),
                compact_decimal(y),
                compact_decimal(z)
            )
        },
    )
}

const fn palette_line(display: DisplayMode) -> &'static str {
    match display {
        DisplayMode::Neutral => "PALETTE NEUTRAL SOURCE-ALPHA",
        DisplayMode::Elevation => "PALETTE ELEVATION LOW > MID > HIGH",
        DisplayMode::Rgb => "PALETTE RAW SOURCE RGB",
        DisplayMode::Intensity => "PALETTE INTENSITY LOW > HIGH",
        DisplayMode::Classification => "PALETTE LAS CLASSIFICATION",
    }
}

fn compact(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{}.{:01}M", value / 1_000_000, value % 1_000_000 / 100_000)
    } else if value >= 1_000 {
        format!("{}.{:01}K", value / 1_000, value % 1_000 / 100)
    } else {
        value.to_string()
    }
}

fn compact_decimal(value: f64) -> String {
    if value.abs() >= 10_000.0 {
        format!("{value:.0}")
    } else if value.abs() >= 100.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

#[cfg(test)]
mod tests {
    use point_workspace::RevisionId;

    use super::*;

    fn snapshot(status: Option<ReviewStatus>) -> StatusSnapshot {
        StatusSnapshot {
            display: DisplayMode::Classification,
            projection: ProjectionMode::Orthographic,
            stream: StreamStatus::Steady,
            scene: SceneMetrics {
                logical_points: 12_345_678,
                resident_points: 654_321,
                sampled_resident_batches: 7,
                complete_resident_batches: 3,
                ..SceneMetrics::default()
            },
            drawn_points: 600_000,
            selected: status,
            resident_highlights: 42,
            orientation: "UP",
            scale_world_units: 125.25,
            cursor_world: Some([6_378_137.25, 13_756_432.5, 120.0]),
        }
    }

    #[test]
    fn primary_status_is_bounded_and_contains_required_non_color_facts() {
        let lines = snapshot(Some(ReviewStatus::Selected {
            revision: RevisionId::from_bytes([7; 32]).unwrap(),
            points: 50,
        }))
        .lines();

        assert!(lines.iter().all(|line| line.len() <= MAX_STATUS_COLUMNS));
        assert!(lines.iter().any(|line| line.contains("PUNCTRA")));
        assert!(lines.iter().any(|line| line.contains("COVERAGE")));
        assert!(lines.iter().any(|line| line.contains("QUERY COMPLETION")));
        assert!(lines.iter().any(|line| line.contains("SELECTED 50")));
        assert!(lines.iter().any(|line| line.contains("X CLEAR")));
        assert!(lines.iter().any(|line| line.contains("CURSOR X")));
        assert!(lines.iter().any(|line| line.contains("PALETTE")));
    }

    #[test]
    fn stale_selection_has_an_explicit_recovery_action() {
        let lines = snapshot(Some(ReviewStatus::SelectionStale {
            revision: RevisionId::from_bytes([8; 32]).unwrap(),
            points: 75,
        }))
        .lines();

        assert!(lines.iter().any(|line| line.contains("STALE")));
        assert!(lines.iter().any(|line| line.contains("RERUN OR CLEAR")));
    }

    #[test]
    fn package_version_has_one_truthful_source() {
        let first = snapshot(None).lines().remove(0);
        assert!(first.contains(&env!("CARGO_PKG_VERSION").to_ascii_uppercase()));
        assert!(!first.contains("V0.11"));
    }

    #[test]
    fn stream_state_distinguishes_loading_settling_steady_and_paused() {
        let loading = SceneMetrics {
            logical_points: 1_000,
            ..SceneMetrics::default()
        };
        assert_eq!(
            StreamStatus::from_facts(loading, false, false, false),
            StreamStatus::Loading
        );

        let resident = SceneMetrics {
            logical_points: 1_000,
            resident_batches: 1,
            resident_points: 100,
            ..SceneMetrics::default()
        };
        assert_eq!(
            StreamStatus::from_facts(resident, false, true, false),
            StreamStatus::Settling
        );
        assert_eq!(
            StreamStatus::from_facts(resident, false, false, true),
            StreamStatus::Settling
        );
        assert_eq!(
            StreamStatus::from_facts(resident, false, false, false),
            StreamStatus::Steady
        );
        assert_eq!(
            StreamStatus::from_facts(loading, true, true, true),
            StreamStatus::LoadsPaused
        );
    }

    #[test]
    fn authored_fixture_coverage_is_named_in_the_primary_status() {
        let mut authored = snapshot(None);
        authored.scene.authored_resident_batches = 583;
        authored.scene.authored_resident_points = 596_992;

        let lines = authored.lines();
        let coverage = lines
            .iter()
            .find(|line| line.starts_with("COVERAGE"))
            .unwrap();
        assert!(coverage.contains("SAMPLED 7"));
        assert!(coverage.contains("COMPLETE 3"));
        assert!(coverage.contains("AUTHORED 583"));
        assert!(coverage.len() <= MAX_STATUS_COLUMNS);
    }
}
