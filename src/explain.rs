use crate::{
    ActivityId, Assignment, CompiledProblem, Conflict, ConflictSeverity, EntityRef, ResourceId,
    Score, ScoreComponent, Solution, TimeWindow,
};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use unifier::VariableId;

#[derive(Debug, Clone, PartialEq)]
pub struct Bottleneck {
    pub entity: EntityRef,
    pub name: String,
    pub required: u64,
    pub available: u64,
    pub utilization: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub feasibility_percent: f64,
    pub bottlenecks: Vec<Bottleneck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveEvaluation {
    pub is_feasible: bool,
    pub hard_violations: Vec<Conflict>,
    pub score_delta: Score,
    pub warnings: Vec<Conflict>,
    pub explanations: Vec<String>,
    /// The score components of the *candidate* plan, so callers can explain a move per
    /// stable category instead of reducing it to one averaged number (plan 25, B4).
    pub score_components: Vec<ScoreComponent>,
}

/// One requested change in an atomic batch: a move or a swap. Every request in a batch is
/// evaluated against the *same* starting solution, so a batch is all-or-nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeRequest {
    /// Moves `activity` to `window`; the duration must stay unchanged.
    Move {
        activity: ActivityId,
        window: TimeWindow,
    },
    /// Exchanges the time windows of two activities — the only atomic way to express it.
    Swap {
        first: ActivityId,
        second: ActivityId,
    },
}

/// The result of evaluating a whole batch of [`ChangeRequest`]s at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvaluation {
    pub is_feasible: bool,
    pub hard_violations: Vec<Conflict>,
    pub warnings: Vec<Conflict>,
    pub score_delta: Score,
    pub score_components: Vec<ScoreComponent>,
    pub explanations: Vec<String>,
    /// The plan the batch would produce — the only place a change is materialised, so
    /// callers never have to re-implement move/swap semantics themselves. It is built even
    /// for a refused batch (the UI wants to show what was attempted); it must therefore
    /// **never** be persisted without checking `is_feasible` first.
    pub candidate: Solution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub activity: ActivityId,
    pub score_delta: Score,
    pub warnings: Vec<Conflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentChange {
    Added(Assignment),
    Removed(Assignment),
    Changed {
        before: Assignment,
        after: Assignment,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolutionComparison {
    pub changes: Vec<AssignmentChange>,
    pub score_delta: Score,
}

impl CompiledProblem {
    /// Estimates pressure from domain sizes and resource/participant demand without solving.
    pub fn analyze(&self) -> Analysis {
        let horizon = self
            .problem
            .academic_period
            .map(|period| period.window)
            .or_else(|| {
                let start = self
                    .problem
                    .activities
                    .iter()
                    .map(|activity| activity.allowed_window().start)
                    .min()?;
                let end = self
                    .problem
                    .activities
                    .iter()
                    .map(|activity| activity.allowed_window().end)
                    .max()?;
                Some(TimeWindow::new(start, end))
            });
        let horizon_duration = horizon.and_then(TimeWindow::duration).unwrap_or(0);
        let mut required_by_resource: BTreeMap<ResourceId, u64> = BTreeMap::new();
        for activity in &self.problem.activities {
            for requirement in activity.requirements() {
                let candidates = self
                    .problem
                    .resources
                    .iter()
                    .filter(|resource| {
                        let in_pool = requirement.pool().is_none_or(|pool_id| {
                            self.problem
                                .resource_pools
                                .iter()
                                .find(|pool| pool.id == pool_id)
                                .is_some_and(|pool| pool.resources.contains(&resource.id()))
                        });
                        requirement
                            .exact_resource()
                            .is_none_or(|id| id == resource.id())
                            && requirement
                                .resource_type()
                                .is_none_or(|kind| kind == resource.resource_type())
                            && resource.capacity() >= requirement.minimum_capacity()
                            && requirement
                                .required_features()
                                .is_subset(resource.features())
                            && (requirement.candidates().is_empty()
                                || requirement.candidates().contains(&resource.id()))
                            && in_pool
                    })
                    .collect::<Vec<_>>();
                let divisor = u64::try_from(candidates.len()).unwrap_or(1).max(1);
                let demand = activity
                    .duration()
                    .saturating_mul(u64::from(requirement.units()))
                    .div_ceil(divisor);
                for resource in candidates {
                    *required_by_resource.entry(resource.id()).or_default() += demand;
                }
            }
        }
        let mut bottlenecks = self
            .problem
            .resources
            .iter()
            .map(|resource| {
                let required = required_by_resource
                    .get(&resource.id())
                    .copied()
                    .unwrap_or_default();
                let available = horizon_duration.saturating_mul(u64::from(resource.capacity()));
                Bottleneck {
                    entity: EntityRef::Resource(resource.id()),
                    name: resource.name().to_string(),
                    required,
                    available,
                    utilization: ratio(required, available),
                }
            })
            .collect::<Vec<_>>();
        let mut required_by_participant = BTreeMap::new();
        for activity in self.internal.activities.values() {
            for participant in activity.participants() {
                *required_by_participant.entry(*participant).or_insert(0u64) += activity.duration();
            }
        }
        bottlenecks.extend(self.problem.participants.iter().map(|participant| {
            let required = required_by_participant
                .get(&participant.id())
                .copied()
                .unwrap_or_default();
            Bottleneck {
                entity: EntityRef::Participant(participant.id()),
                name: participant.name().to_string(),
                required,
                available: horizon_duration,
                utilization: ratio(required, horizon_duration),
            }
        }));
        bottlenecks.sort_by(|left, right| right.utilization.total_cmp(&left.utilization));
        let maximum = bottlenecks
            .first()
            .map_or(0.0, |bottleneck| bottleneck.utilization);
        Analysis {
            feasibility_percent: (100.0 / maximum.max(1.0)).clamp(0.0, 100.0),
            bottlenecks,
        }
    }

    pub fn evaluate_move(
        &self,
        solution: &Solution,
        activity: ActivityId,
        proposed: TimeWindow,
    ) -> MoveEvaluation {
        let Some(activity_model) = self
            .problem
            .activities
            .iter()
            .find(|candidate| candidate.id() == activity)
        else {
            return invalid_move(activity, "unknown activity");
        };
        if proposed.duration() != Some(activity_model.duration())
            || proposed.start < activity_model.allowed_window().start
            || proposed.end > activity_model.allowed_window().end
        {
            return invalid_move(activity, "proposed window is outside the activity domain");
        }
        let mut committed = self.internal.assignment_map(solution);
        let old_score = self.internal.score_for(&committed);
        let (start, end) = self.internal.variables_for(activity);
        let conflicts = self
            .internal
            .check_incremental(&committed, &[(start, proposed.start), (end, proposed.end)]);
        committed.insert(start, proposed.start);
        committed.insert(end, proposed.end);
        let new_score = self.internal.score_for(&committed);
        let hard_violations = conflicts
            .iter()
            .filter(|conflict| conflict.severity == ConflictSeverity::Blocking)
            .cloned()
            .collect::<Vec<_>>();
        let warnings = conflicts
            .iter()
            .filter(|conflict| conflict.severity == ConflictSeverity::Advisory)
            .cloned()
            .collect::<Vec<_>>();
        MoveEvaluation {
            is_feasible: hard_violations.is_empty(),
            hard_violations,
            score_delta: subtract_score(new_score, old_score),
            warnings,
            explanations: conflicts
                .into_iter()
                .map(|conflict| conflict.message)
                .collect(),
            score_components: self.internal.score_components(&committed),
        }
    }

    /// Evaluates several changes **atomically** against one starting solution (plan 25,
    /// B4). Move and swap can be mixed in one batch; everything is applied to a private
    /// copy of the assignment, so a half-applied state can never be observed and the
    /// starting solution is never mutated. The first invalid request aborts the whole
    /// batch with a blocking `ActivityDomain` violation — as does a starting solution that
    /// names an activity the problem no longer contains (a stale solution).
    pub fn evaluate_changes(
        &self,
        solution: &Solution,
        changes: &[ChangeRequest],
    ) -> ChangeEvaluation {
        if let Some(stale) = solution
            .assignments
            .iter()
            .find(|assignment| !self.internal.knows_activity(assignment.activity))
        {
            return invalid_change(
                stale.activity,
                "solution refers to an activity that is not part of the problem",
                solution,
            );
        }
        let mut committed = self.internal.assignment_map(solution);
        let windows = solution
            .assignments
            .iter()
            .map(|assignment| (assignment.activity, assignment.window))
            .collect::<BTreeMap<_, _>>();
        let mut updates: Vec<(VariableId, i64)> = Vec::new();
        for change in changes {
            let (activity, window) = match change {
                ChangeRequest::Move { activity, window } => (*activity, *window),
                ChangeRequest::Swap { first, second } => {
                    let (Some(first_window), Some(second_window)) =
                        (windows.get(first), windows.get(second))
                    else {
                        return invalid_change(
                            *first,
                            "swap refers to an activity that is not part of the solution",
                            solution,
                        );
                    };
                    if first_window.duration() != second_window.duration() {
                        return invalid_change(
                            *first,
                            "swap requires two activities of equal duration",
                            solution,
                        );
                    }
                    updates.extend(self.window_updates(*first, *second_window));
                    updates.extend(self.window_updates(*second, *first_window));
                    continue;
                }
            };
            let Some(model) = self
                .problem
                .activities
                .iter()
                .find(|candidate| candidate.id() == activity)
            else {
                return invalid_change(activity, "unknown activity", solution);
            };
            if window.duration() != Some(model.duration())
                || window.start < model.allowed_window().start
                || window.end > model.allowed_window().end
            {
                return invalid_change(
                    activity,
                    "proposed window is outside the activity domain",
                    solution,
                );
            }
            updates.extend(self.window_updates(activity, window));
        }
        let old_score = self.internal.score_for(&committed);
        let conflicts = self.internal.check_incremental(&committed, &updates);
        for (variable, value) in updates {
            committed.insert(variable, value);
        }
        let new_score = self.internal.score_for(&committed);
        let score_components = self.internal.score_components(&committed);
        let candidate = Solution {
            assignments: solution
                .assignments
                .iter()
                .map(|assignment| {
                    let (start, end) = self.internal.variables_for(assignment.activity);
                    Assignment {
                        activity: assignment.activity,
                        window: TimeWindow::new(
                            committed
                                .get(&start)
                                .copied()
                                .unwrap_or(assignment.window.start),
                            committed
                                .get(&end)
                                .copied()
                                .unwrap_or(assignment.window.end),
                        ),
                        resources: assignment.resources.clone(),
                        participants: assignment.participants.clone(),
                    }
                })
                .collect(),
            score: new_score,
            score_components: score_components.clone(),
        };
        ChangeEvaluation::from_conflicts(
            conflicts,
            subtract_score(new_score, old_score),
            score_components,
            candidate,
        )
    }

    fn window_updates(&self, activity: ActivityId, window: TimeWindow) -> [(VariableId, i64); 2] {
        let (start, end) = self.internal.variables_for(activity);
        [(start, window.start), (end, window.end)]
    }

    pub fn suggest(&self, solution: &Solution, proposed: TimeWindow) -> Vec<Suggestion> {
        let mut suggestions = self
            .problem
            .activities
            .iter()
            .filter_map(|activity| {
                let evaluation = self.evaluate_move(solution, activity.id(), proposed);
                evaluation.is_feasible.then_some(Suggestion {
                    activity: activity.id(),
                    score_delta: evaluation.score_delta,
                    warnings: evaluation.warnings,
                })
            })
            .collect::<Vec<_>>();
        suggestions.sort_by_key(|suggestion| Reverse(suggestion.score_delta));
        suggestions
    }
}

fn invalid_move(activity: ActivityId, message: &str) -> MoveEvaluation {
    let conflict = Conflict {
        severity: ConflictSeverity::Blocking,
        constraint_name: "ActivityDomain".to_string(),
        involved: vec![activity],
        entity: Some(EntityRef::Activity(activity)),
        message: message.to_string(),
    };
    MoveEvaluation {
        is_feasible: false,
        hard_violations: vec![conflict.clone()],
        score_delta: Score::default(),
        warnings: Vec::new(),
        explanations: vec![conflict.message],
        score_components: Vec::new(),
    }
}

fn invalid_change(activity: ActivityId, message: &str, solution: &Solution) -> ChangeEvaluation {
    let conflict = Conflict {
        severity: ConflictSeverity::Blocking,
        constraint_name: "ActivityDomain".to_string(),
        involved: vec![activity],
        entity: Some(EntityRef::Activity(activity)),
        message: message.to_string(),
    };
    ChangeEvaluation {
        is_feasible: false,
        hard_violations: vec![conflict.clone()],
        warnings: Vec::new(),
        score_delta: Score::default(),
        score_components: Vec::new(),
        explanations: vec![conflict.message],
        // A refused batch changes nothing — the candidate is the baseline itself.
        candidate: solution.clone(),
    }
}

impl ChangeEvaluation {
    fn from_conflicts(
        conflicts: Vec<Conflict>,
        score_delta: Score,
        score_components: Vec<ScoreComponent>,
        candidate: Solution,
    ) -> Self {
        let hard_violations = conflicts
            .iter()
            .filter(|conflict| conflict.severity == ConflictSeverity::Blocking)
            .cloned()
            .collect::<Vec<_>>();
        let warnings = conflicts
            .iter()
            .filter(|conflict| conflict.severity == ConflictSeverity::Advisory)
            .cloned()
            .collect::<Vec<_>>();
        Self {
            is_feasible: hard_violations.is_empty(),
            hard_violations,
            warnings,
            score_delta,
            score_components,
            explanations: conflicts
                .into_iter()
                .map(|conflict| conflict.message)
                .collect(),
            candidate,
        }
    }
}

pub fn compare(before: &Solution, after: &Solution) -> SolutionComparison {
    let before_by_id = before
        .assignments
        .iter()
        .map(|assignment| (assignment.activity, assignment))
        .collect::<BTreeMap<_, _>>();
    let after_by_id = after
        .assignments
        .iter()
        .map(|assignment| (assignment.activity, assignment))
        .collect::<BTreeMap<_, _>>();
    let mut ids = before_by_id.keys().copied().collect::<Vec<_>>();
    ids.extend(after_by_id.keys().copied());
    ids.sort_unstable();
    ids.dedup();
    let changes = ids
        .into_iter()
        .filter_map(|id| match (before_by_id.get(&id), after_by_id.get(&id)) {
            (Some(before), Some(after)) if before != after => Some(AssignmentChange::Changed {
                before: (*before).clone(),
                after: (*after).clone(),
            }),
            (Some(before), None) => Some(AssignmentChange::Removed((*before).clone())),
            (None, Some(after)) => Some(AssignmentChange::Added((*after).clone())),
            _ => None,
        })
        .collect();
    SolutionComparison {
        changes,
        score_delta: subtract_score(after.score, before.score),
    }
}

fn ratio(required: u64, available: u64) -> f64 {
    if available == 0 {
        if required == 0 { 0.0 } else { 1.0 }
    } else {
        required as f64 / available as f64
    }
}

fn subtract_score(after: Score, before: Score) -> Score {
    Score {
        hard: after.hard.saturating_sub(before.hard),
        strong: after.strong.saturating_sub(before.strong),
        medium: after.medium.saturating_sub(before.medium),
        weak: after.weak.saturating_sub(before.weak),
        soft: after.soft.saturating_sub(before.soft),
    }
}
