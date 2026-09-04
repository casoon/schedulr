use crate::{
    CompiledProblem, ScoreLevel, ScoreRule, ScoreRuleKind, Solution, SolveResult, compile,
};
use std::time::Duration;
use unifier::{LnsSolver, SolverOptions};

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub change_penalty: i64,
    pub destroy_fraction: f64,
    pub time_limit: Duration,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            change_penalty: 1_000_000,
            destroy_fraction: 0.3,
            time_limit: Duration::from_secs(1),
        }
    }
}

impl CompiledProblem {
    /// Re-solves around a baseline using a strong penalty per changed assignment and LNS.
    pub fn repair(&self, baseline: &Solution, options: &RepairOptions) -> SolveResult {
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
        let Ok(compiled) = compile(&problem) else {
            return self.solve();
        };
        let baseline_assignment = compiled.internal.assignment_map(baseline);
        let outcome = LnsSolver::new(options.destroy_fraction).solve_from(
            &compiled.internal.graph,
            &baseline_assignment,
            &SolverOptions {
                time_limit: Some(options.time_limit),
                ..SolverOptions::default()
            },
        );
        compiled.internal.solve_result(outcome)
    }
}
