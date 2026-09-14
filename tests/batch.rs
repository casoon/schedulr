use schedulr::{
    AbortReason, Activity, ActivityId, CancellationToken, ConflictSeverity, Resource, ResourceId,
    ResourceRequirement, SchedulingProblem, SolveOptions, SolveStatus, TimeWindow, compile,
};

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
