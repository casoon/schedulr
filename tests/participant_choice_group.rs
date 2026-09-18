//! Integration tests for the `ParticipantChoiceGroup` hard constraint (plan 33.1).
//!
//! A group of activities must select the **same** participant for their candidate requirement.
//! The primitive is domain-neutral: it is what lets a course's several terms be taught by one
//! teacher without the engine knowing what a teacher or a course is.

use schedulr::{
    Activity, ActivityId, Assignment, Participant, ParticipantChoiceGroup, ParticipantId,
    ParticipantRequirement, SchedulingProblem, Solution, SolveStatus, TimeWindow, compile,
};

/// A one-hour activity whose teacher is chosen from `candidates`, free in `[0, 2)`.
fn activity(id: u64, candidates: &[u64]) -> Activity {
    let mut requirement = ParticipantRequirement::matching();
    for candidate in candidates {
        requirement = requirement.with_candidate(ParticipantId(*candidate));
    }
    Activity::new(
        ActivityId(id),
        format!("activity {id}"),
        TimeWindow::new(0, 2),
        1,
    )
    .with_participant_requirement(requirement)
}

fn teachers() -> Vec<Participant> {
    vec![
        Participant::new(ParticipantId(1), "Teacher 1"),
        Participant::new(ParticipantId(2), "Teacher 2"),
    ]
}

/// Both activities must pick the same teacher, even though each alone is free to pick either.
#[test]
fn the_group_forces_the_same_participant_across_its_activities() {
    let problem = SchedulingProblem::new(
        vec![],
        teachers(),
        vec![activity(1, &[1, 2]), activity(2, &[1, 2])],
    )
    .with_participant_choice_group(ParticipantChoiceGroup::new(vec![
        ActivityId(1),
        ActivityId(2),
    ]));

    let solution = compile(&problem)
        .expect("the group has a common candidate")
        .solve()
        .solution
        .expect("a plan exists");

    let chosen = |id: u64| {
        solution
            .assignments
            .iter()
            .find(|assignment| assignment.activity == ActivityId(id))
            .expect("activity is assigned")
            .participants
            .clone()
    };
    assert_eq!(
        chosen(1),
        chosen(2),
        "both activities pick the same teacher"
    );
}

/// Both activities share the candidate set, but each candidate is unavailable in exactly one of
/// the two windows: no single teacher can carry both, so the group is infeasible.
#[test]
fn the_group_is_infeasible_when_no_common_participant_has_time() {
    let first = activity(1, &[1, 2]);
    let second = Activity::new(ActivityId(2), "activity 2", TimeWindow::new(1, 2), 1)
        .with_participant_requirement(
            ParticipantRequirement::matching()
                .with_candidate(ParticipantId(1))
                .with_candidate(ParticipantId(2)),
        );
    let teacher_one = Participant::new(ParticipantId(1), "Teacher 1").with_unavailable_range(1, 1);
    let teacher_two = Participant::new(ParticipantId(2), "Teacher 2").with_unavailable_range(0, 0);

    let problem =
        SchedulingProblem::new(vec![], vec![teacher_one, teacher_two], vec![first, second])
            .with_participant_choice_group(ParticipantChoiceGroup::new(vec![
                ActivityId(1),
                ActivityId(2),
            ]));

    assert_eq!(
        compile(&problem).unwrap().solve().status,
        SolveStatus::Infeasible
    );
}

/// An empty intersection is refused at compile time with an explanation instead of an opaque
/// infeasibility.
#[test]
fn an_empty_intersection_is_a_compile_error() {
    let problem = SchedulingProblem::new(
        vec![],
        teachers(),
        vec![activity(1, &[1]), activity(2, &[2])],
    )
    .with_participant_choice_group(ParticipantChoiceGroup::new(vec![
        ActivityId(1),
        ActivityId(2),
    ]));

    let error = compile(&problem).expect_err("no common candidate");
    assert!(
        error
            .messages()
            .iter()
            .any(|message| message.contains("no common candidate")),
        "{:?}",
        error.messages()
    );
}

/// A solution that splits the group across two teachers is reported under the stable constraint
/// name in `check`.
#[test]
fn a_split_choice_is_a_named_conflict_in_check() {
    let compiled = compile(
        &SchedulingProblem::new(
            vec![],
            teachers(),
            vec![activity(1, &[1, 2]), activity(2, &[1, 2])],
        )
        .with_participant_choice_group(ParticipantChoiceGroup::new(vec![
            ActivityId(1),
            ActivityId(2),
        ])),
    )
    .unwrap();

    let solution = Solution {
        assignments: vec![
            Assignment::new(ActivityId(1), TimeWindow::new(0, 1))
                .with_participant(ParticipantId(1)),
            Assignment::new(ActivityId(2), TimeWindow::new(1, 2))
                .with_participant(ParticipantId(2)),
        ],
        score: Default::default(),
        score_components: vec![],
    };
    let check = compiled.check(&solution);

    assert!(!check.is_feasible);
    assert!(
        check
            .hard_violations
            .iter()
            .any(|conflict| conflict.constraint_name == "ParticipantChoiceGroup"),
        "{:?}",
        check.hard_violations
    );
}

/// The same split is reported through the atomic change-evaluation path.
#[test]
fn a_split_choice_is_a_named_conflict_in_evaluate_changes() {
    let compiled = compile(
        &SchedulingProblem::new(
            vec![],
            teachers(),
            vec![activity(1, &[1, 2]), activity(2, &[1, 2])],
        )
        .with_participant_choice_group(ParticipantChoiceGroup::new(vec![
            ActivityId(1),
            ActivityId(2),
        ])),
    )
    .unwrap();

    let baseline = Solution {
        assignments: vec![
            Assignment::new(ActivityId(1), TimeWindow::new(0, 1))
                .with_participant(ParticipantId(1)),
            Assignment::new(ActivityId(2), TimeWindow::new(1, 2))
                .with_participant(ParticipantId(2)),
        ],
        score: Default::default(),
        score_components: vec![],
    };
    let evaluation = compiled.evaluate_changes(
        &baseline,
        &[schedulr::ChangeRequest::Move {
            activity: ActivityId(1),
            window: TimeWindow::new(0, 1),
        }],
    );

    assert!(!evaluation.is_feasible);
    assert!(
        evaluation
            .hard_violations
            .iter()
            .any(|conflict| conflict.constraint_name == "ParticipantChoiceGroup"),
        "{:?}",
        evaluation.hard_violations
    );
}
