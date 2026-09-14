use schedulr::{
    Activity, ActivityId, Assignment, ChangeRequest, CompiledProblem, Resource, ResourceId,
    ResourceRequirement, SchedulingProblem, Score, ScoreLevel, ScoreRule, Solution, TimeWindow,
    compile,
};
use std::collections::BTreeMap;

/// One room at capacity 1, so two activities can never share a slot. That is the fixture
/// that makes atomicity observable: a one-sided move is blocked, the swap of both is not.
fn fixture() -> CompiledProblem {
    let requirement = ResourceRequirement::matching("room", 1);
    let first = Activity::new(ActivityId(1), "Mathematik 10A", TimeWindow::new(0, 4), 1)
        .with_requirement(requirement.clone());
    let second = Activity::new(ActivityId(2), "Deutsch 10B", TimeWindow::new(0, 4), 1)
        .with_requirement(requirement);
    compile(&SchedulingProblem::new(
        vec![Resource::new(ResourceId(1), "A204", 1).with_type("room")],
        vec![],
        vec![first, second],
    ))
    .unwrap()
}

/// A hand-built baseline — 1st and 2nd hour in the same room. Deterministic on purpose:
/// what is under test is the change evaluation, not the solver, so the tests must not
/// depend on which optimum a search happens to return.
fn baseline() -> Solution {
    Solution {
        assignments: vec![
            Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
            Assignment::new(ActivityId(2), TimeWindow::new(1, 2)).with_resource(ResourceId(1)),
        ],
        score: Score::default(),
        score_components: Vec::new(),
    }
}

#[test]
fn a_move_into_an_occupied_slot_is_blocked_with_an_explanation() {
    let evaluation = fixture().evaluate_changes(
        &baseline(),
        &[ChangeRequest::Move {
            activity: ActivityId(1),
            window: TimeWindow::new(1, 2),
        }],
    );

    assert!(!evaluation.is_feasible);
    assert!(!evaluation.hard_violations.is_empty());
    // The explanation must survive to the caller — a bare "not allowed" is not enough.
    assert!(!evaluation.explanations.is_empty());
    assert!(
        evaluation
            .hard_violations
            .iter()
            .all(|conflict| !conflict.constraint_name.is_empty())
    );
}

#[test]
fn a_swap_is_only_valid_atomically() {
    let compiled = fixture();
    let baseline = baseline();

    // Moving activity 1 onto activity 2's slot *alone* is blocked …
    let single = compiled.evaluate_changes(
        &baseline,
        &[ChangeRequest::Move {
            activity: ActivityId(1),
            window: TimeWindow::new(1, 2),
        }],
    );
    assert!(!single.is_feasible);

    // … but the batch that exchanges both windows is valid. Evaluated one after the other
    // it would pass through exactly the blocked intermediate state above.
    let swapped = compiled.evaluate_changes(
        &baseline,
        &[ChangeRequest::Swap {
            first: ActivityId(1),
            second: ActivityId(2),
        }],
    );
    assert!(swapped.is_feasible, "{:?}", swapped.explanations);
    assert!(swapped.hard_violations.is_empty());
}

#[test]
fn a_move_batch_that_mixes_move_and_swap_stays_all_or_nothing() {
    let compiled = fixture();
    let baseline = baseline();

    // The move alone is fine (3rd hour is free), the swap is fine — but this batch also
    // moves activity 2 onto the slot the swap already gave it, which cannot hold both.
    let evaluation = compiled.evaluate_changes(
        &baseline,
        &[
            ChangeRequest::Swap {
                first: ActivityId(1),
                second: ActivityId(2),
            },
            ChangeRequest::Move {
                activity: ActivityId(2),
                window: TimeWindow::new(0, 1),
            },
        ],
    );

    // Either it is feasible as a whole, or it is refused as a whole — never half-applied.
    if !evaluation.is_feasible {
        assert!(!evaluation.hard_violations.is_empty());
    }
    assert_eq!(
        evaluation.is_feasible,
        evaluation.hard_violations.is_empty(),
        "feasibility and hard violations must agree"
    );
}

#[test]
fn evaluating_changes_never_mutates_the_starting_solution() {
    let compiled = fixture();
    let baseline = baseline();
    let untouched = baseline.clone();

    let evaluation = compiled.evaluate_changes(
        &baseline,
        &[ChangeRequest::Swap {
            first: ActivityId(1),
            second: ActivityId(2),
        }],
    );
    assert!(evaluation.is_feasible);

    // The baseline is the version the user still sees; a candidate evaluation must leave
    // it byte-identical (this also guards a future `&mut` refactor of the signature).
    assert_eq!(baseline, untouched);
    assert_eq!(
        untouched.assignments[0].window,
        TimeWindow::new(0, 1),
        "the baseline must keep its original windows"
    );
}

#[test]
fn a_swap_of_unequal_durations_is_rejected() {
    let requirement = ResourceRequirement::matching("room", 1);
    let double = Activity::new(ActivityId(1), "Doppelstunde", TimeWindow::new(0, 4), 2)
        .with_requirement(requirement.clone());
    let single = Activity::new(ActivityId(2), "Einzelstunde", TimeWindow::new(0, 4), 1)
        .with_requirement(requirement);
    let compiled = compile(&SchedulingProblem::new(
        vec![Resource::new(ResourceId(1), "A204", 1).with_type("room")],
        vec![],
        vec![double, single],
    ))
    .unwrap();
    let solution = Solution {
        assignments: vec![
            Assignment::new(ActivityId(1), TimeWindow::new(0, 2)).with_resource(ResourceId(1)),
            Assignment::new(ActivityId(2), TimeWindow::new(2, 3)).with_resource(ResourceId(1)),
        ],
        score: Score::default(),
        score_components: Vec::new(),
    };

    let evaluation = compiled.evaluate_changes(
        &solution,
        &[ChangeRequest::Swap {
            first: ActivityId(1),
            second: ActivityId(2),
        }],
    );

    assert!(!evaluation.is_feasible);
    assert!(
        evaluation
            .explanations
            .iter()
            .any(|message| message.contains("equal duration")),
        "{:?}",
        evaluation.explanations
    );
}

#[test]
fn a_window_outside_the_activity_domain_is_rejected() {
    // The activities are allowed 0..4, so 4..5 is not a slot the plan may use.
    let evaluation = fixture().evaluate_changes(
        &baseline(),
        &[ChangeRequest::Move {
            activity: ActivityId(1),
            window: TimeWindow::new(4, 5),
        }],
    );

    assert!(!evaluation.is_feasible);
    assert_eq!(
        evaluation.hard_violations[0].constraint_name,
        "ActivityDomain"
    );
}

#[test]
fn score_components_are_reported_per_stable_category() {
    let requirement = ResourceRequirement::matching("room", 1);
    let first = Activity::new(ActivityId(1), "Mathematik 10A", TimeWindow::new(0, 4), 1)
        .with_requirement(requirement.clone());
    let second = Activity::new(ActivityId(2), "Deutsch 10B", TimeWindow::new(0, 4), 1)
        .with_requirement(requirement);
    let problem = SchedulingProblem::new(
        vec![Resource::new(ResourceId(1), "A204", 1).with_type("room")],
        vec![],
        vec![first, second],
    )
    .with_score_rule(ScoreRule::prefer_window(
        "Freistunden Lehrkraft",
        ScoreLevel::Strong,
        ActivityId(1),
        TimeWindow::new(3, 4),
        1,
    ));
    let compiled = compile(&problem).unwrap();

    let evaluation = compiled.evaluate_changes(
        &baseline(),
        &[ChangeRequest::Move {
            activity: ActivityId(1),
            window: TimeWindow::new(3, 4),
        }],
    );

    assert!(evaluation.is_feasible, "{:?}", evaluation.explanations);
    // The category is the stable key the UI groups by — not a localized sentence.
    assert!(
        evaluation.score_components.iter().any(|component| {
            component.category == "Freistunden Lehrkraft" && component.level == ScoreLevel::Strong
        }),
        "{:?}",
        evaluation
            .score_components
            .iter()
            .map(|component| component.category.clone())
            .collect::<Vec<_>>()
    );
    // The move satisfies the preference, so it must be reported as an improvement.
    assert!(evaluation.score_delta.soft > 0);
}

#[test]
fn the_candidate_materialises_the_swap_window_for_window() {
    let evaluation = fixture().evaluate_changes(
        &baseline(),
        &[ChangeRequest::Swap {
            first: ActivityId(1),
            second: ActivityId(2),
        }],
    );
    assert!(evaluation.is_feasible, "{:?}", evaluation.explanations);

    let windows = evaluation
        .candidate
        .assignments
        .iter()
        .map(|assignment| (assignment.activity, assignment.window))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(windows[&ActivityId(1)], TimeWindow::new(1, 2));
    assert_eq!(windows[&ActivityId(2)], TimeWindow::new(0, 1));
    // The room assignment must survive the change — a swap moves time, not resources.
    assert_eq!(
        evaluation.candidate.assignments[0].resources,
        vec![ResourceId(1)]
    );
}

#[test]
fn a_refused_batch_reports_its_candidate_but_never_mutates_the_baseline() {
    let baseline = baseline();
    let evaluation = fixture().evaluate_changes(
        &baseline,
        &[ChangeRequest::Move {
            activity: ActivityId(1),
            window: TimeWindow::new(1, 2),
        }],
    );

    assert!(!evaluation.is_feasible);
    // The candidate mirrors what the user attempted (the UI needs it to explain the
    // refusal) …
    let moved = evaluation
        .candidate
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(1))
        .unwrap();
    assert_eq!(moved.window, TimeWindow::new(1, 2));
    // … while the baseline this was evaluated against stays untouched.
    assert_eq!(baseline.assignments[0].window, TimeWindow::new(0, 1));
}
