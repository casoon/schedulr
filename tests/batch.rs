use schedulr::{
    AbortReason, Activity, ActivityId, CancellationToken, ConflictSeverity, Participant,
    ParticipantId, Resource, ResourceId, ResourceRequirement, SchedulingProblem, ScoreLevel,
    SoftGoal, SoftGoalKind, SolveOptions, SolveStatus, TimeWindow, compile,
};
use std::time::Duration;

#[test]
fn compile_solve_and_explain_a_fixed_hard_conflict() {
    let room = Resource::new(ResourceId(7), "Physics lab", 1);
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(10, 20), 10)
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(15, 25), 10)
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let compiled = compile(&SchedulingProblem::new(
        vec![room],
        vec![],
        vec![first, second],
    ))
    .expect("problem compiles");

    let result = compiled.solve();
    assert_eq!(result.status, SolveStatus::Infeasible);
    let conflicts = compiled.explain(&result);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].severity, ConflictSeverity::Blocking);
    assert_eq!(conflicts[0].constraint_name, "NoOverlap");
    assert_eq!(conflicts[0].involved, vec![ActivityId(1), ActivityId(2)]);
    assert!(conflicts[0].message.contains("Physics lab"));
}

#[test]
fn batch_solver_returns_domain_assignments() {
    let room = Resource::new(ResourceId(1), "Room", 1);
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 5), 2)
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 5), 2)
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let compiled = compile(&SchedulingProblem::new(
        vec![room],
        vec![],
        vec![first, second],
    ))
    .expect("problem compiles");

    let result = compiled.solve();
    assert_eq!(result.status, SolveStatus::Feasible);
    assert_eq!(result.solution.unwrap().assignments.len(), 2);
}

#[test]
fn solve_with_an_already_cancelled_token_aborts_instead_of_searching() {
    let room = Resource::new(ResourceId(1), "Room", 1);
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 5), 2)
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 5), 2)
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let compiled = compile(&SchedulingProblem::new(
        vec![room],
        vec![],
        vec![first, second],
    ))
    .expect("problem compiles");

    let token = CancellationToken::new();
    token.cancel();
    let result = compiled.solve_with(&SolveOptions {
        cancellation_token: Some(token),
        ..SolveOptions::default()
    });

    assert_eq!(result.status, SolveStatus::Aborted(AbortReason::Cancelled));
    assert!(result.solution.is_none());
}

/// A run whose budget ends mid-optimization still returns a schedule, rather than reporting
/// "time is up, nothing here".
///
/// Two participants share one unary resource, each wanting its own activities packed together
/// while the resource forces them to interleave, and a second goal pulls the same activities
/// apart. No arrangement satisfies all of it, so the optimizer cannot close the gap between its
/// bound and the best schedule within the limit — `statistics.optimal` stays false — while a
/// feasible arrangement is easy to reach. Whatever the optimizer does with the remaining time,
/// the schedule has to survive it.
#[test]
fn a_schedule_survives_a_budget_too_small_to_optimize() {
    let room = Resource::new(ResourceId(1), "Room", 1);
    let first = Participant::new(ParticipantId(1), "P1");
    let second = Participant::new(ParticipantId(2), "P2");
    let activities: Vec<Activity> = (1..=16)
        .map(|id| {
            let owner = if id % 2 == 0 { first.id() } else { second.id() };
            Activity::new(ActivityId(id), format!("a{id}"), TimeWindow::new(0, 60), 2)
                .with_requirement(ResourceRequirement::new(room.id(), 1))
                .with_participant(owner)
        })
        .collect();
    let compiled = compile(
        &SchedulingProblem::new(vec![room], vec![first, second], activities)
            .with_soft_goal(SoftGoal::new(
                "idle",
                ScoreLevel::Weak,
                1,
                SoftGoalKind::MinimizeParticipantIdle,
            ))
            .with_soft_goal(SoftGoal::new(
                "spread",
                ScoreLevel::Weak,
                1,
                SoftGoalKind::SpreadActivityOverDays,
            )),
    )
    .expect("problem compiles");

    let result = compiled.solve_with(&SolveOptions {
        time_limit: Some(Duration::from_millis(150)),
        ..SolveOptions::default()
    });

    assert!(
        !result.statistics.optimal,
        "the instance is meant to outlast the budget, otherwise it proves nothing about being cut off"
    );
    assert!(
        !matches!(result.status, SolveStatus::Aborted(_)),
        "a reachable schedule must not be discarded because the optimizer ran out of time, got {:?}",
        result.status
    );
    let solution = result.solution.expect("a schedule is returned");
    // Never take the solver's own word for feasibility: check the schedule independently.
    let check = compiled.check(&solution);
    assert!(
        check.is_feasible,
        "the returned schedule must hold every hard constraint, violated: {:?}",
        check.hard_violations
    );
}

/// Unsatisfiable hard constraints are *proven* unsatisfiable even when goals are configured.
///
/// Goals cannot make an over-subscribed resource fit, so the answer has to be the same proof the
/// goal-free problem gets — not an "out of time" from an optimizer that never reached a complete
/// assignment to report on.
#[test]
fn hard_conflicts_are_proven_infeasible_even_with_goals_configured() {
    let room = Resource::new(ResourceId(7), "Lab", 1);
    let participant = Participant::new(ParticipantId(1), "P");
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(10, 20), 10)
        .with_requirement(ResourceRequirement::new(room.id(), 1))
        .with_participant(participant.id());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(15, 25), 10)
        .with_requirement(ResourceRequirement::new(room.id(), 1))
        .with_participant(participant.id());
    let compiled = compile(
        &SchedulingProblem::new(vec![room], vec![participant], vec![first, second]).with_soft_goal(
            SoftGoal::new(
                "packing",
                ScoreLevel::Weak,
                1,
                SoftGoalKind::MinimizeParticipantIdle,
            ),
        ),
    )
    .expect("problem compiles");

    let result = compiled.solve_with(&SolveOptions {
        time_limit: Some(Duration::from_secs(5)),
        ..SolveOptions::default()
    });

    assert_eq!(result.status, SolveStatus::Infeasible);
    assert!(result.solution.is_none());
}
