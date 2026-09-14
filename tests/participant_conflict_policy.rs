//! Participant collisions are detected either way — the policy only decides the verdict
//! (plan 26, §1). The default must stay exactly what a booking desk relies on, and a domain
//! that cannot tolerate a double-booked person must be able to say so without a second
//! detection path.

use schedulr::{
    Activity, ActivityId, Assignment, ChangeRequest, CompiledProblem, EntityRef, Participant,
    ParticipantConflictPolicy, ParticipantId, ParticipantPool, ParticipantPoolId,
    ParticipantRequirement, SchedulingProblem, Score, Solution, TimeWindow, compile,
};

/// Two lessons of one person, no resource in the way — the participant is the *only* possible
/// collision. Both windows are wide enough to hold the other lesson's slot, so the collision is
/// decided by the move itself and not by an exhausted domain.
fn one_person_two_lessons(policy: ParticipantConflictPolicy) -> CompiledProblem {
    let problem = SchedulingProblem::new(
        vec![],
        vec![Participant::new(ParticipantId(1), "Alex")],
        vec![
            Activity::new(ActivityId(1), "Mathematik 10A", TimeWindow::new(0, 4), 1)
                .with_participant(ParticipantId(1)),
            Activity::new(ActivityId(2), "Deutsch 10B", TimeWindow::new(0, 4), 1)
                .with_participant(ParticipantId(1)),
        ],
    )
    .with_participant_conflict_policy(policy);
    compile(&problem).expect("a valid problem")
}

/// The same two lessons, but the person is only a *candidate* the solver may choose — the
/// requirement is satisfied from a pool, so the collision is found by the capacity constraint
/// on the selected participant, not by a fixed participant relation.
fn one_person_two_lessons_from_pool(policy: ParticipantConflictPolicy) -> CompiledProblem {
    let problem = SchedulingProblem::new(
        vec![],
        vec![Participant::new(ParticipantId(1), "Alex")],
        vec![
            Activity::new(ActivityId(1), "Mathematik 10A", TimeWindow::new(0, 4), 1)
                .with_participant_requirement(ParticipantRequirement::from_pool(
                    ParticipantPoolId(1),
                )),
            Activity::new(ActivityId(2), "Deutsch 10B", TimeWindow::new(0, 4), 1)
                .with_participant_requirement(ParticipantRequirement::from_pool(
                    ParticipantPoolId(1),
                )),
        ],
    )
    .with_participant_pool(ParticipantPool::new(
        ParticipantPoolId(1),
        "teachers",
        [ParticipantId(1)],
    ))
    .with_participant_conflict_policy(policy);
    compile(&problem).expect("a valid problem")
}

/// Hand-built baseline: the two lessons sit next to each other, this person is on both.
fn baseline() -> Solution {
    Solution {
        assignments: vec![
            Assignment::new(ActivityId(1), TimeWindow::new(0, 1))
                .with_participant(ParticipantId(1)),
            Assignment::new(ActivityId(2), TimeWindow::new(1, 2))
                .with_participant(ParticipantId(1)),
        ],
        score: Score::default(),
        score_components: Vec::new(),
    }
}

/// Move the first lesson onto the second one's slot.
fn move_onto_the_other_lesson() -> [ChangeRequest; 1] {
    [ChangeRequest::Move {
        activity: ActivityId(1),
        window: TimeWindow::new(1, 2),
    }]
}

#[test]
fn the_default_policy_is_advisory() {
    // Guards the compatibility promise the docs make: not setting a policy keeps the old verdict.
    assert_eq!(
        SchedulingProblem::default().participant_conflict_policy,
        ParticipantConflictPolicy::Advisory
    );
}

#[test]
fn a_double_booked_person_is_only_reported_by_default() {
    let evaluation = one_person_two_lessons(ParticipantConflictPolicy::Advisory)
        .evaluate_changes(&baseline(), &move_onto_the_other_lesson());

    assert!(evaluation.is_feasible, "{:?}", evaluation.explanations);
    assert!(evaluation.hard_violations.is_empty());
    assert!(
        evaluation
            .warnings
            .iter()
            .any(|conflict| conflict.entity == Some(EntityRef::Participant(ParticipantId(1)))),
        "the collision must still be reported, just not block: {:?}",
        evaluation.warnings
    );
}

#[test]
fn a_blocking_policy_refuses_exactly_the_same_arrangement() {
    let advisory = one_person_two_lessons(ParticipantConflictPolicy::Advisory)
        .evaluate_changes(&baseline(), &move_onto_the_other_lesson());
    let blocking = one_person_two_lessons(ParticipantConflictPolicy::Blocking)
        .evaluate_changes(&baseline(), &move_onto_the_other_lesson());

    assert!(!blocking.is_feasible);
    assert!(
        blocking
            .hard_violations
            .iter()
            .any(|conflict| conflict.entity == Some(EntityRef::Participant(ParticipantId(1))))
    );
    assert!(blocking.warnings.is_empty());
    // Only the verdict may differ — the same rule, on the same two activities, with the same text.
    let reported = advisory.warnings.first().expect("reported by default");
    let refused = blocking.hard_violations.first().expect("refused here");
    assert_eq!(reported.constraint_name, refused.constraint_name);
    assert_eq!(reported.message, refused.message);
    assert_eq!(reported.involved, refused.involved);
}

#[test]
fn a_policy_that_does_not_apply_leaves_everything_else_alone() {
    // A resource collision is blocking under either policy — the policy names participants only.
    let problem = SchedulingProblem::new(
        vec![schedulr::Resource::new(schedulr::ResourceId(1), "A204", 1)],
        vec![Participant::new(ParticipantId(1), "Alex")],
        vec![
            Activity::new(ActivityId(1), "Mathematik 10A", TimeWindow::new(0, 4), 1)
                .with_requirement(schedulr::ResourceRequirement::new(
                    schedulr::ResourceId(1),
                    1,
                )),
            Activity::new(ActivityId(2), "Deutsch 10B", TimeWindow::new(0, 4), 1).with_requirement(
                schedulr::ResourceRequirement::new(schedulr::ResourceId(1), 1),
            ),
        ],
    )
    .with_participant_conflict_policy(ParticipantConflictPolicy::Blocking);
    let evaluation = compile(&problem)
        .expect("a valid problem")
        .evaluate_changes(
            &Solution {
                assignments: vec![
                    Assignment::new(ActivityId(1), TimeWindow::new(0, 1))
                        .with_resource(schedulr::ResourceId(1)),
                    Assignment::new(ActivityId(2), TimeWindow::new(1, 2))
                        .with_resource(schedulr::ResourceId(1)),
                ],
                score: Score::default(),
                score_components: Vec::new(),
            },
            &move_onto_the_other_lesson(),
        );

    assert!(!evaluation.is_feasible);
    assert!(
        evaluation
            .hard_violations
            .iter()
            .any(|conflict| conflict.entity == Some(EntityRef::Resource(schedulr::ResourceId(1))))
    );
}

#[test]
fn a_person_selected_from_a_pool_is_governed_by_the_same_policy() {
    // The same collision found by the other mechanism (capacity on the selected participant)
    // must honour the policy too — otherwise the verdict would depend on how the requirement
    // was written rather than on what actually happens.
    let advisory = one_person_two_lessons_from_pool(ParticipantConflictPolicy::Advisory)
        .evaluate_changes(&baseline(), &move_onto_the_other_lesson());
    assert!(advisory.is_feasible, "{:?}", advisory.explanations);
    assert!(
        advisory
            .warnings
            .iter()
            .any(|conflict| conflict.entity == Some(EntityRef::Participant(ParticipantId(1))))
    );

    let blocking = one_person_two_lessons_from_pool(ParticipantConflictPolicy::Blocking)
        .evaluate_changes(&baseline(), &move_onto_the_other_lesson());
    assert!(!blocking.is_feasible);
    assert!(
        blocking
            .hard_violations
            .iter()
            .any(|conflict| conflict.entity == Some(EntityRef::Participant(ParticipantId(1))))
    );
}
