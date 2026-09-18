//! E0.1: `CompiledProblem::check` is the whole-solution counterpart of the per-activity
//! no-op probe — same constraints, same verdicts, but one call and no mutation (plan 31).

use schedulr::{
    Activity, ActivityId, Assignment, CompiledProblem, Conflict, Participant,
    ParticipantConflictPolicy, ParticipantId, Resource, ResourceId, ResourceRequirement,
    SchedulingProblem, Solution, TimeWindow, compile,
};
use std::collections::BTreeSet;

/// One typed room with a single candidate: the requirement is resolved by the solver, so each
/// activity carries a presence variable and an `ExactlyOne` over it — the flexible case the
/// no-op probes and the whole-solution check must agree on.
fn flexible_room() -> CompiledProblem {
    let requirement = ResourceRequirement::matching("room", 1);
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 3), 1)
        .with_requirement(requirement.clone());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 3), 1)
        .with_requirement(requirement);
    compile(&SchedulingProblem::new(
        vec![Resource::new(ResourceId(1), "A204", 1).with_type("room")],
        vec![],
        vec![first, second],
    ))
    .unwrap()
}

fn valid_assignments() -> Vec<Assignment> {
    vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
        Assignment::new(ActivityId(2), TimeWindow::new(1, 2)).with_resource(ResourceId(1)),
    ]
}

fn solution(assignments: Vec<Assignment>) -> Solution {
    Solution {
        assignments,
        score: schedulr::Score::default(),
        score_components: Vec::new(),
    }
}

fn one_person_two_lessons(policy: ParticipantConflictPolicy) -> CompiledProblem {
    compile(
        &SchedulingProblem::new(
            vec![],
            vec![Participant::new(ParticipantId(1), "Alex")],
            vec![
                Activity::new(ActivityId(1), "first", TimeWindow::new(0, 2), 1)
                    .with_participant(ParticipantId(1)),
                Activity::new(ActivityId(2), "second", TimeWindow::new(0, 2), 1)
                    .with_participant(ParticipantId(1)),
            ],
        )
        .with_participant_conflict_policy(policy),
    )
    .unwrap()
}

fn conflict_keys(conflicts: &[Conflict]) -> BTreeSet<String> {
    conflicts
        .iter()
        .map(|conflict| format!("{conflict:?}"))
        .collect()
}

#[test]
fn a_valid_solution_has_no_conflicts_and_keeps_its_score() {
    let compiled = flexible_room();
    let solved = compiled.solve().solution.expect("fixture is feasible");

    let check = compiled.check(&solved);

    assert!(check.is_feasible);
    assert!(check.hard_violations.is_empty());
    assert!(check.warnings.is_empty());
    assert!(check.explanations.is_empty());
    assert_eq!(check.score, solved.score);
}

#[test]
fn a_resource_collision_is_a_blocking_violation() {
    let compiled = flexible_room();
    let overlapping = solution(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
        Assignment::new(ActivityId(2), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
    ]);

    let check = compiled.check(&overlapping);

    assert!(!check.is_feasible);
    assert!(
        check.hard_violations.iter().any(|conflict| {
            conflict.constraint_name == "AlternativeResourceCapacity"
                || conflict.constraint_name == "NoOverlap"
        }),
        "{:?}",
        check.hard_violations
    );
}

#[test]
fn a_participant_collision_follows_the_configured_policy() {
    let overlapping = solution(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_participant(ParticipantId(1)),
        Assignment::new(ActivityId(2), TimeWindow::new(0, 1)).with_participant(ParticipantId(1)),
    ]);

    let advisory = one_person_two_lessons(ParticipantConflictPolicy::Advisory).check(&overlapping);
    assert!(advisory.is_feasible);
    assert!(advisory.hard_violations.is_empty());
    let reported = advisory
        .warnings
        .iter()
        .filter(|conflict| {
            conflict.entity == Some(schedulr::EntityRef::Participant(ParticipantId(1)))
        })
        .count();
    assert!(reported >= 1, "{:?}", advisory.warnings);

    let blocking = one_person_two_lessons(ParticipantConflictPolicy::Blocking).check(&overlapping);
    assert!(!blocking.is_feasible);
    assert!(
        blocking
            .hard_violations
            .iter()
            .any(|conflict| conflict.entity
                == Some(schedulr::EntityRef::Participant(ParticipantId(1)))),
        "{:?}",
        blocking.hard_violations
    );
    assert!(blocking.warnings.is_empty());
}

#[test]
fn an_unknown_activity_is_reported_per_activity_without_panicking() {
    let compiled = flexible_room();
    let mut stale = solution(valid_assignments());
    stale
        .assignments
        .push(Assignment::new(ActivityId(99), TimeWindow::new(2, 3)).with_resource(ResourceId(1)));

    let check = compiled.check(&stale);

    assert!(!check.is_feasible);
    let unknown: Vec<_> = check
        .hard_violations
        .iter()
        .filter(|conflict| conflict.constraint_name == "ActivityDomain")
        .collect();
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].involved, vec![ActivityId(99)]);
}

#[test]
fn a_missing_assignment_is_reported_as_unassigned() {
    let compiled = flexible_room();
    // Only activity 1 is placed; activity 2 is left out of the solution.
    let partial = solution(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
    ]);

    let check = compiled.check(&partial);

    assert!(!check.is_feasible);
    let unassigned: Vec<_> = check
        .hard_violations
        .iter()
        .filter(|conflict| conflict.constraint_name == "Unassigned")
        .collect();
    assert_eq!(unassigned.len(), 1);
    assert_eq!(unassigned[0].involved, vec![ActivityId(2)]);
}

#[test]
fn check_does_not_mutate_the_solution() {
    let compiled = flexible_room();
    let candidate = solution(valid_assignments());
    let untouched = candidate.clone();

    let _ = compiled.check(&candidate);

    assert_eq!(candidate, untouched);
}

/// The whole-solution check must equal the union of the per-activity no-op probes
/// `evaluate_move` runs — the semantics `validate_version` relied on before E0.1.
#[test]
fn check_equals_the_union_of_no_op_probes() {
    let compiled = flexible_room();
    let conflicting = solution(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
        Assignment::new(ActivityId(2), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
    ]);

    let check = compiled.check(&conflicting);

    let mut probed = Vec::new();
    for assignment in &conflicting.assignments {
        let evaluation =
            compiled.evaluate_move(&conflicting, assignment.activity, assignment.window);
        probed.extend(evaluation.hard_violations);
        probed.extend(evaluation.warnings);
    }
    let probed_keys = conflict_keys(&probed);
    let mut checked_keys = conflict_keys(&check.hard_violations);
    checked_keys.extend(conflict_keys(&check.warnings));

    assert_eq!(probed_keys, checked_keys);
}
