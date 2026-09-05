use schedulr::{
    AcademicPeriod, Activity, ActivityId, ActivityRelation, ActivityRelationConstraint,
    AssignmentChange, BreakTemplate, DayTemplate, GroupMembership, Participant, ParticipantGroup,
    ParticipantGroupId, ParticipantId, ParticipantPool, ParticipantPoolId, ParticipantRequirement,
    RepairOptions, Resource, ResourceId, ResourcePool, ResourcePoolId, ResourceRequirement,
    ScheduleTemplate, SchedulingProblem, ScoreLevel, ScoreRule, SlotTemplate, SolveStatus,
    TimeWindow, compare, compile,
};
use std::time::Duration;

#[test]
fn matching_resources_choose_non_conflicting_alternatives() {
    let first_room = Resource::new(ResourceId(1), "Lab A", 1)
        .with_type("room")
        .with_feature("lab");
    let second_room = Resource::new(ResourceId(2), "Lab B", 1)
        .with_type("room")
        .with_feature("lab");
    let requirement = ResourceRequirement::matching("room", 1).with_feature("lab");
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1)
        .with_requirement(requirement.clone());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 1), 1)
        .with_requirement(requirement);

    let result = compile(&SchedulingProblem::new(
        vec![first_room, second_room],
        vec![],
        vec![first, second],
    ))
    .unwrap()
    .solve();
    let solution = result.solution.unwrap();

    assert_ne!(
        solution.assignments[0].resources,
        solution.assignments[1].resources
    );
}

#[test]
fn infeasible_alternative_resources_have_an_explanation() {
    let rooms = vec![
        Resource::new(ResourceId(1), "Lab A", 1).with_type("room"),
        Resource::new(ResourceId(2), "Lab B", 1).with_type("room"),
    ];
    let activities = (1..=3)
        .map(|id| {
            Activity::new(
                ActivityId(id),
                format!("activity {id}"),
                TimeWindow::new(0, 1),
                1,
            )
            .with_requirement(ResourceRequirement::matching("room", 1))
        })
        .collect();
    let compiled = compile(&SchedulingProblem::new(rooms, vec![], activities)).unwrap();
    let result = compiled.solve();

    assert_eq!(result.status, SolveStatus::Infeasible);
    let conflicts = compiled.explain(&result);
    assert!(!conflicts.is_empty());
    assert!(conflicts.iter().any(|conflict| {
        conflict.constraint_name == "AlternativeResourceCapacity"
            && conflict.message.contains("unresolved alternatives")
    }));
}

#[test]
fn participant_pool_chooses_a_non_conflicting_participant() {
    let first_teacher = Participant::new(ParticipantId(1), "Müller");
    let second_teacher = Participant::new(ParticipantId(2), "Schmidt");
    let fixed = Activity::new(ActivityId(1), "fixed", TimeWindow::new(0, 1), 1)
        .with_participant(first_teacher.id());
    let flexible = Activity::new(ActivityId(2), "flexible", TimeWindow::new(0, 1), 1)
        .with_participant_requirement(ParticipantRequirement::from_pool(ParticipantPoolId(1)));
    let problem = SchedulingProblem::new(
        vec![],
        vec![first_teacher, second_teacher],
        vec![fixed, flexible],
    )
    .with_participant_pool(ParticipantPool::new(
        ParticipantPoolId(1),
        "math teachers",
        [ParticipantId(1), ParticipantId(2)],
    ));

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let assignment = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap();
    assert_eq!(assignment.participants, vec![ParticipantId(2)]);
}

#[test]
fn named_resource_pool_limits_the_candidate_set() {
    let first = Resource::new(ResourceId(1), "Pool member", 1);
    let second = Resource::new(ResourceId(2), "Outside", 1);
    let activity = Activity::new(ActivityId(1), "pooled", TimeWindow::new(0, 1), 1)
        .with_requirement(ResourceRequirement::from_pool(ResourcePoolId(1), 1));
    let problem =
        SchedulingProblem::new(vec![first, second], vec![], vec![activity]).with_resource_pool(
            ResourcePool::new(ResourcePoolId(1), "preferred rooms", [ResourceId(1)]),
        );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    assert_eq!(solution.assignments[0].resources, vec![ResourceId(1)]);
}

#[test]
fn hierarchical_and_overlapping_group_memberships_block_double_booking() {
    let learner = Participant::new(ParticipantId(1), "Learner");
    let parent = ParticipantGroup::new(ParticipantGroupId(1), "Year");
    let subgroup = ParticipantGroup::new(ParticipantGroupId(2), "Lab group");
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1)
        .with_participant_group(parent.id);
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 1), 1)
        .with_participant(learner.id());
    let problem = SchedulingProblem::new(vec![], vec![learner], vec![first, second])
        .with_participant_group(parent)
        .with_participant_group(subgroup)
        .with_group_membership(GroupMembership::subgroup(
            ParticipantGroupId(1),
            ParticipantGroupId(2),
        ))
        .with_group_membership(GroupMembership::participant(
            ParticipantGroupId(2),
            ParticipantId(1),
        ));

    assert_eq!(
        compile(&problem).unwrap().solve().status,
        SolveStatus::Infeasible
    );
}

#[test]
fn periodic_calendar_handles_cycles_and_absolute_exceptions() {
    let template = ScheduleTemplate::new(10)
        .with_day(DayTemplate::new(0).with_slot(SlotTemplate::new("first", 1, 1)))
        .with_unavailable_range(1, 1);
    let activity = Activity::new(ActivityId(1), "A week or B week", TimeWindow::new(0, 20), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![activity]).with_calendar(
        AcademicPeriod {
            window: TimeWindow::new(0, 20),
        },
        template,
    );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    assert_eq!(solution.assignments[0].window.start, 11);
}

#[test]
fn break_template_blocks_activities_that_would_overlap_it() {
    // "first" (offset 0, duration 1) overlaps the 0..2 break; "second"
    // (offset 2, duration 1) starts right after it, so only offset 2 remains.
    let template = ScheduleTemplate::new(10).with_day(
        DayTemplate::new(0)
            .with_slot(SlotTemplate::new("first", 0, 1))
            .with_slot(SlotTemplate::new("second", 2, 1))
            .with_break(BreakTemplate {
                name: "Pause".to_string(),
                window: TimeWindow::new(0, 2),
            }),
    );
    let activity = Activity::new(ActivityId(1), "after the break", TimeWindow::new(0, 4), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![activity]).with_calendar(
        AcademicPeriod {
            window: TimeWindow::new(0, 4),
        },
        template,
    );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    assert_eq!(solution.assignments[0].window.start, 2);
}

#[test]
fn resource_unavailable_range_blocks_the_early_slot() {
    // AllowedTime via a fixed (exact) resource requirement: the gym is unavailable
    // 0..=3, so the earliest usable start is 4, not the naive earliest, 0.
    let gym = Resource::new(ResourceId(1), "Sporthalle", 1).with_unavailable_range(0, 3);
    let activity = Activity::new(ActivityId(1), "Sport", TimeWindow::new(0, 10), 1)
        .with_requirement(ResourceRequirement::new(ResourceId(1), 1));
    let problem = SchedulingProblem::new(vec![gym], vec![], vec![activity]);

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    assert_eq!(solution.assignments[0].window.start, 4);
}

#[test]
fn participant_availability_makes_the_solver_pick_the_other_candidate() {
    // Availability via a flexible participant requirement: the only feasible slot
    // (start 0) is unavailable for Krüger, so the solver must pick Bauer instead —
    // proving the calendar restriction is applied per-candidate, not globally.
    let unavailable = Participant::new(ParticipantId(1), "Krüger").with_unavailable_range(0, 0);
    let available = Participant::new(ParticipantId(2), "Bauer");
    let activity = Activity::new(ActivityId(1), "Mathe", TimeWindow::new(0, 1), 1)
        .with_participant_requirement(
            ParticipantRequirement::matching()
                .with_candidate(ParticipantId(1))
                .with_candidate(ParticipantId(2)),
        );
    let problem = SchedulingProblem::new(vec![], vec![unavailable, available], vec![activity]);

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    assert_eq!(solution.assignments[0].participants, vec![ParticipantId(2)]);
}

#[test]
fn same_start_relation_forces_two_unrelated_activities_to_align() {
    // Activity 1 is pinned to start 3; activity 2 is otherwise free (and would
    // naturally start at 0). SameStart must drag it to 3 instead.
    let pinned = Activity::new(ActivityId(1), "pinned", TimeWindow::new(3, 4), 1);
    let free = Activity::new(ActivityId(2), "free", TimeWindow::new(0, 5), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![pinned, free]).with_relation(
        ActivityRelationConstraint::new(ActivityId(1), ActivityId(2), ActivityRelation::SameStart),
    );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let free_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    assert_eq!(free_start, 3);
}

#[test]
fn consecutive_relation_starts_the_second_activity_exactly_when_the_first_ends() {
    let first = Activity::new(ActivityId(1), "theory", TimeWindow::new(0, 2), 2);
    let second = Activity::new(ActivityId(2), "lab", TimeWindow::new(0, 10), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![first, second]).with_relation(
        ActivityRelationConstraint::new(
            ActivityId(1),
            ActivityId(2),
            ActivityRelation::Consecutive,
        ),
    );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let second_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    assert_eq!(second_start, 2);
}

#[test]
fn precedence_relation_enforces_a_minimum_gap() {
    let first = Activity::new(ActivityId(1), "theory", TimeWindow::new(0, 1), 1);
    let second = Activity::new(ActivityId(2), "lab", TimeWindow::new(0, 10), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![first, second]).with_relation(
        ActivityRelationConstraint::new(
            ActivityId(1),
            ActivityId(2),
            ActivityRelation::Precedence { min_gap: 3 },
        ),
    );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let second_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    assert_eq!(second_start, 4);
}

#[test]
fn no_overlap_relation_keeps_two_unrelated_activities_apart() {
    // Neither activity shares a resource or participant, so nothing else would
    // stop them from occupying the same slot; only the relation does.
    let first = Activity::new(ActivityId(1), "assembly", TimeWindow::new(0, 2), 2);
    let second = Activity::new(ActivityId(2), "free period", TimeWindow::new(0, 10), 2);
    let problem = SchedulingProblem::new(vec![], vec![], vec![first, second]).with_relation(
        ActivityRelationConstraint::new(ActivityId(1), ActivityId(2), ActivityRelation::NoOverlap),
    );

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let second_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    assert_eq!(second_start, 2);
}

#[test]
fn tiered_scores_drive_solving_and_explain_move_deltas() {
    let activity = Activity::new(ActivityId(1), "math", TimeWindow::new(0, 2), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![activity])
        .with_score_rule(ScoreRule::prefer_window(
            "important morning",
            ScoreLevel::Strong,
            ActivityId(1),
            TimeWindow::new(0, 1),
            1,
        ))
        .with_score_rule(ScoreRule::prefer_window(
            "nice afternoon",
            ScoreLevel::Medium,
            ActivityId(1),
            TimeWindow::new(1, 2),
            1_000,
        ));
    let compiled = compile(&problem).unwrap();
    let solution = compiled.solve().solution.unwrap();

    assert_eq!(solution.assignments[0].window.start, 0);
    assert_eq!(solution.score_components.len(), 2);
    let moved = compiled.evaluate_move(&solution, ActivityId(1), TimeWindow::new(1, 2));
    assert!(moved.is_feasible);
    assert_eq!(moved.score_delta.strong, -1);
    assert_eq!(moved.score_delta.medium, 1_000);
    assert_eq!(compiled.suggest(&solution, TimeWindow::new(1, 2)).len(), 1);

    let mut changed = solution.clone();
    changed.assignments[0].window = TimeWindow::new(1, 2);
    assert!(matches!(
        compare(&solution, &changed).changes.as_slice(),
        [AssignmentChange::Changed { .. }]
    ));
}

#[test]
fn analyze_runs_without_solving_and_repair_minimizes_changes() {
    let old_room = Resource::new(ResourceId(1), "Shared room", 2);
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 2), 1)
        .with_requirement(ResourceRequirement::new(old_room.id(), 1));
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 2), 1)
        .with_requirement(ResourceRequirement::new(old_room.id(), 1));
    let activities = vec![first, second];
    let old = compile(&SchedulingProblem::new(
        vec![old_room],
        vec![],
        activities.clone(),
    ))
    .unwrap();
    let baseline = old.solve().solution.unwrap();
    let current = compile(&SchedulingProblem::new(
        vec![Resource::new(ResourceId(1), "Shared room", 1)],
        vec![],
        activities,
    ))
    .unwrap();

    assert!(!current.analyze().bottlenecks.is_empty());
    let repaired = current
        .repair(
            &baseline,
            &RepairOptions {
                time_limit: Duration::from_millis(100),
                destroy_fraction: 0.9,
                ..RepairOptions::default()
            },
        )
        .solution
        .unwrap();
    assert_eq!(compare(&baseline, &repaired).changes.len(), 1);
}
