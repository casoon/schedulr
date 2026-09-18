use crate::batch::{StabilityAssignment, StabilityBaseline, compile_with_stability};
use crate::{
    ActivityId, CancellationToken, CompiledProblem, ScoreLevel, ScoreRule, ScoreRuleKind, Solution,
    SolveResult,
};
use std::collections::BTreeSet;
use std::time::Duration;
use unifier::{LnsSolver, SolverOptions};

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub change_penalty: i64,
    /// `Strong` penalty for a resource/participant selection deviating from the baseline
    /// (plan 31, E0.2). `None` means "use [`Self::change_penalty`]", so a caller that only tunes
    /// the start penalty does not have to name this one as well (plan: "Default = change_penalty").
    pub assignment_change_penalty: Option<i64>,
    /// Activities whose time, participants and resources are pinned to their baseline values
    /// (plan 31, E0.2).
    pub frozen: BTreeSet<ActivityId>,
    /// Handle a caller can use to interrupt the repair search from another thread/task.
    pub cancellation: Option<CancellationToken>,
    pub destroy_fraction: f64,
    pub time_limit: Duration,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            change_penalty: 1_000_000,
            assignment_change_penalty: None,
            frozen: BTreeSet::new(),
            cancellation: None,
            destroy_fraction: 0.3,
            time_limit: Duration::from_secs(1),
        }
    }
}

/// Whether [`CompiledProblem::repair_with_outcome`] actually repaired from the baseline or had to
/// fall back to a fresh solve because the stability-augmented problem did not compile
/// (plan 31, E0.2). A caller that shows a plan as a "repair" must check this first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairOutcome {
    /// The large neighbourhood search ran from the baseline.
    Repaired,
    /// Compilation failed and [`CompiledProblem::solve`] produced a fresh plan instead.
    FellBackToSolve,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairResult {
    pub outcome: RepairOutcome,
    pub result: SolveResult,
}

impl CompiledProblem {
    /// Re-solves around a baseline using a strong penalty per changed assignment and LNS.
    ///
    /// The returned [`SolveResult`] is exactly [`Self::repair_with_outcome`]'s `result`; use that
    /// method when the caller must distinguish a repair from a fallback solve.
    pub fn repair(&self, baseline: &Solution, options: &RepairOptions) -> SolveResult {
        self.repair_with_outcome(baseline, options).result
    }

    /// Like [`Self::repair`], but reports whether a fresh solve was substituted because the
    /// stability-augmented problem failed to compile (plan 31, E0.2).
    pub fn repair_with_outcome(
        &self,
        baseline: &Solution,
        options: &RepairOptions,
    ) -> RepairResult {
        let mut problem = self.problem.clone();
        problem
            .score_rules
            .extend(baseline.assignments.iter().map(|assignment| ScoreRule {
                category: "stability".to_string(),
                level: ScoreLevel::Strong,
                activity: assignment.activity,
                weight: options.change_penalty,
                kind: ScoreRuleKind::KeepStart(assignment.window.start),
            }));
        let stability = StabilityBaseline {
            assignments: baseline
                .assignments
                .iter()
                .map(|assignment| {
                    (
                        assignment.activity,
                        StabilityAssignment {
                            start: assignment.window.start,
                            resources: assignment.resources.iter().copied().collect(),
                            participants: assignment.participants.iter().copied().collect(),
                        },
                    )
                })
                .collect(),
            assignment_change_penalty: options
                .assignment_change_penalty
                .unwrap_or(options.change_penalty),
            frozen: options.frozen.clone(),
        };
        let Ok(compiled) = compile_with_stability(&problem, Some(&stability)) else {
            return RepairResult {
                outcome: RepairOutcome::FellBackToSolve,
                result: self.solve(),
            };
        };
        let baseline_assignment = compiled.internal.assignment_map(baseline);
        let outcome = LnsSolver::new(options.destroy_fraction).solve_from(
            &compiled.internal.graph,
            &baseline_assignment,
            &SolverOptions {
                time_limit: Some(options.time_limit),
                cancellation_token: options.cancellation.clone(),
                ..SolverOptions::default()
            },
        );
        RepairResult {
            outcome: RepairOutcome::Repaired,
            result: compiled.internal.solve_result(outcome),
        }
    }
}
