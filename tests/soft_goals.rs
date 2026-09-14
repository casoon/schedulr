//! Integration tests for the four problem-level soft goals (`SoftGoal` / `SoftGoalKind`).
//!
//! Each test proves two things: the goal's contribution shows up in the solution's
//! `score_components` under the configured category (with `activity == None`), and the goal moves
//! the score in the intended direction — a "bad" arrangement scores strictly worse than a "good"
//! one, and the solver, given the freedom, picks the good one.

use schedulr::{
    AcademicPeriod, Activity, ActivityId, DayTemplate, GroupMembership, Participant,
    ParticipantGroup, ParticipantGroupId, ParticipantId, Resource, ResourceId, ResourceRequirement,
    ScheduleTemplate, SchedulingProblem, ScoreComponent, ScoreLevel, SlotTemplate, SoftGoal,
    SoftGoalKind, Solution, TimeWindow, compile,
};
use std::collections::BTreeSet;

/// Two "days" within a 10-unit cycle: day 0 covers `[0, 5)`, day 1 covers `[5, 10)`.
fn two_day_template() -> ScheduleTemplate {
    ScheduleTemplate::new(10)
        .with_day(
            DayTemplate::new(0)
                .with_slot(SlotTemplate::new("d0-s0", 0, 1))
                .with_slot(SlotTemplate::new("d0-s1", 1, 1))
                .with_slot(SlotTemplate::new("d0-s2", 2, 1))
                .with_slot(SlotTemplate::new("d0-s3", 3, 1))
                .with_slot(SlotTemplate::new("d0-s4", 4, 1)),
        )
        .with_day(
            DayTemplate::new(5)
                .with_slot(SlotTemplate::new("d1-s0", 0, 1))
                .with_slot(SlotTemplate::new("d1-s1", 1, 1)),
        )
}

fn period(start: i64, end: i64) -> AcademicPeriod {
    AcademicPeriod {
        window: TimeWindow::new(start, end),
    }
}

fn start_of(solution: &Solution, activity: ActivityId) -> i64 {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == activity)
        .unwrap_or_else(|| panic!("activity {activity} is assigned"))
        .window
        .start
}

fn resources_of(solution: &Solution, activity: ActivityId) -> BTreeSet<ResourceId> {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == activity)
        .unwrap_or_else(|| panic!("activity {activity} is assigned"))
        .resources
        .iter()
        .copied()
        .collect()
}

fn component<'a>(solution: &'a Solution, category: &str) -> &'a ScoreComponent {
    solution
        .score_components
        .iter()
        .find(|component| component.category == category)
        .unwrap_or_else(|| {
            panic!(
                "score_components must carry the configured category {category:?}: {:?}",
                solution
                    .score_components
                    .iter()
                    .map(|component| component.category.clone())
                    .collect::<Vec<_>>()
            )
        })
}

/// `MinimizeParticipantIdle`: free time between one participant's assignments is penalised, so the
/// solver packs them together.
#[test]
fn minimize_participant_idle_penalises_gaps_and_moves_the_solver() {
    let participant = Participant::new(ParticipantId(1), "P");
    let category = "participant idle";

    // A forced gap of 4 units between [0, 1) and [5, 6) scores negatively.
    let pinned_low = Activity::new(ActivityId(1), "a", TimeWindow::new(0, 1), 1)
        .with_participant(participant.id());
    let pinned_high = Activity::new(ActivityId(2), "b", TimeWindow::new(5, 6), 1)
        .with_participant(participant.id());
    let forced = SchedulingProblem::new(
        vec![],
        vec![participant.clone()],
        vec![pinned_low, pinned_high],
    )
    .with_soft_goal(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        2,
        SoftGoalKind::MinimizeParticipantIdle,
    ));
    let forced = compile(&forced).unwrap().solve().solution.unwrap();
    let forced_component = component(&forced, category);
    assert_eq!(
        forced_component.activity, None,
        "a soft goal is problem-level"
    );
    assert_eq!(forced_component.level, ScoreLevel::Weak);
    assert_eq!(
        forced_component.value, -8,
        "gap 4 * weight 2 must be penalised"
    );
    assert_eq!(forced.score.weak, -8);

    // With a free second activity, the solver closes the gap: it schedules it right before [5, 6).
    let pinned = Activity::new(ActivityId(1), "a", TimeWindow::new(5, 6), 1)
        .with_participant(participant.id());
    let free = Activity::new(ActivityId(2), "b", TimeWindow::new(0, 6), 1)
        .with_participant(participant.id());

    let baseline = SchedulingProblem::new(
        vec![],
        vec![participant.clone()],
        vec![pinned.clone(), free.clone()],
    );
    let baseline = compile(&baseline).unwrap().solve().solution.unwrap();
    assert_eq!(
        start_of(&baseline, ActivityId(2)),
        0,
        "without the goal the earliest start leaves the gap"
    );

    let problem = SchedulingProblem::new(vec![], vec![participant], vec![pinned, free])
        .with_soft_goal(SoftGoal::new(
            category,
            ScoreLevel::Weak,
            1,
            SoftGoalKind::MinimizeParticipantIdle,
        ));
    let compiled = compile(&problem).unwrap();
    let solution = compiled.solve().solution.unwrap();
    assert_eq!(
        start_of(&solution, ActivityId(2)),
        4,
        "the goal packs the free activity directly before the pinned one"
    );
    assert_eq!(component(&solution, category).value, 0);

    // And moving it back to the gapped position strictly lowers the goal's contribution.
    let moved = compiled.evaluate_move(&solution, ActivityId(2), TimeWindow::new(0, 1));
    assert!(moved.is_feasible);
    assert_eq!(
        moved.score_delta.weak, -4,
        "reintroducing the gap must lower the score"
    );
}

/// `MinimizeGroupIdle`: the same idea, but over the aggregate occupancy of a participant group, so
/// gaps spanning *different* members are penalised too.
#[test]
fn minimize_group_idle_aggregates_members_and_moves_the_solver() {
    let first = Participant::new(ParticipantId(1), "P1");
    let second = Participant::new(ParticipantId(2), "P2");
    let group = ParticipantGroup::new(ParticipantGroupId(1), "G");
    let memberships = [
        GroupMembership::participant(group.id, first.id()),
        GroupMembership::participant(group.id, second.id()),
    ];
    let category = "group idle";

    // Each member occupies one end of the day: the group's union has a gap of 4.
    let early =
        Activity::new(ActivityId(1), "a", TimeWindow::new(0, 1), 1).with_participant(first.id());
    let late =
        Activity::new(ActivityId(2), "b", TimeWindow::new(5, 6), 1).with_participant(second.id());
    let forced = SchedulingProblem::new(
        vec![],
        vec![first.clone(), second.clone()],
        vec![early, late],
    )
    .with_participant_group(group.clone())
    .with_group_membership(memberships[0])
    .with_group_membership(memberships[1])
    .with_soft_goal(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        2,
        SoftGoalKind::MinimizeGroupIdle,
    ));
    let forced = compile(&forced).unwrap().solve().solution.unwrap();
    let forced_component = component(&forced, category);
    assert_eq!(forced_component.activity, None);
    assert_eq!(
        forced_component.value, -8,
        "the union [0, 1) + [5, 6) leaves a gap of 4"
    );

    // A free second member can be moved to close the gap.
    let pinned =
        Activity::new(ActivityId(1), "a", TimeWindow::new(5, 6), 1).with_participant(first.id());
    let free =
        Activity::new(ActivityId(2), "b", TimeWindow::new(0, 6), 1).with_participant(second.id());
    let build = |goal: SoftGoal| {
        SchedulingProblem::new(
            vec![],
            vec![first.clone(), second.clone()],
            vec![pinned.clone(), free.clone()],
        )
        .with_participant_group(group.clone())
        .with_group_membership(memberships[0])
        .with_group_membership(memberships[1])
        .with_soft_goal(goal)
    };

    // A per-participant idle goal sees no gap (each member has a single assignment), so it leaves
    // the free activity at its earliest start — proof the group aggregation is doing the work.
    let per_participant = build(SoftGoal::new(
        "participant idle",
        ScoreLevel::Weak,
        1,
        SoftGoalKind::MinimizeParticipantIdle,
    ));
    let per_participant = compile(&per_participant).unwrap().solve().solution.unwrap();
    assert_eq!(start_of(&per_participant, ActivityId(2)), 0);

    let grouped = build(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        1,
        SoftGoalKind::MinimizeGroupIdle,
    ));
    let grouped = compile(&grouped).unwrap().solve().solution.unwrap();
    assert_eq!(
        start_of(&grouped, ActivityId(2)),
        4,
        "the group goal must move the free member next to the other member's assignment"
    );
    assert_eq!(component(&grouped, category).value, 0);
}

/// `RoomStability`: occurrences sharing an activity name prefer the same resource.
#[test]
fn room_stability_reduces_distinct_resources_and_moves_the_solver() {
    let first = Resource::new(ResourceId(1), "R1", 1);
    let second = Resource::new(ResourceId(2), "R2", 1);
    let category = "resource stability";
    let requirement = || {
        ResourceRequirement::matching("resource", 1)
            .with_candidate(first.id())
            .with_candidate(second.id())
    };

    // Forced into the same time slot, the two "math" occurrences must use two distinct resources.
    let crowded = SchedulingProblem::new(
        vec![first.clone(), second.clone()],
        vec![],
        vec![
            Activity::new(ActivityId(1), "math", TimeWindow::new(0, 1), 1)
                .with_requirement(requirement()),
            Activity::new(ActivityId(2), "math", TimeWindow::new(0, 1), 1)
                .with_requirement(requirement()),
        ],
    )
    .with_soft_goal(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        3,
        SoftGoalKind::RoomStability,
    ));
    let crowded = compile(&crowded).unwrap().solve().solution.unwrap();
    let crowded_component = component(&crowded, category);
    assert_eq!(crowded_component.activity, None);
    assert_eq!(
        crowded_component.value, -3,
        "two distinct resources must cost one unit * weight 3"
    );

    // With free slots, the goal keeps both occurrences on a single resource.
    let free_activities = || {
        vec![
            Activity::new(ActivityId(1), "math", TimeWindow::new(0, 4), 1)
                .with_requirement(requirement()),
            Activity::new(ActivityId(2), "math", TimeWindow::new(0, 4), 1)
                .with_requirement(requirement()),
        ]
    };

    // A forced *stable* arrangement (non-overlapping slots) can share one resource and scores 0.
    let stable = SchedulingProblem::new(
        vec![first.clone(), second.clone()],
        vec![],
        vec![
            Activity::new(ActivityId(1), "math", TimeWindow::new(0, 1), 1)
                .with_requirement(requirement()),
            Activity::new(ActivityId(2), "math", TimeWindow::new(1, 2), 1)
                .with_requirement(requirement()),
        ],
    )
    .with_soft_goal(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        3,
        SoftGoalKind::RoomStability,
    ));
    let stable = compile(&stable).unwrap().solve().solution.unwrap();
    assert_eq!(
        component(&stable, category).value,
        0,
        "reusing a single resource must not be penalised"
    );
    assert_eq!(
        resources_of(&stable, ActivityId(1)),
        resources_of(&stable, ActivityId(2)),
        "the solver naturally reuses one resource for non-overlapping occurrences"
    );
    assert!(
        stable.score.weak > crowded.score.weak,
        "the stable arrangement ({}) must score better than the crowded one ({})",
        stable.score.weak,
        crowded.score.weak
    );

    let problem = SchedulingProblem::new(
        vec![first.clone(), second.clone()],
        vec![],
        free_activities(),
    )
    .with_soft_goal(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        1,
        SoftGoalKind::RoomStability,
    ));
    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let first_resource = resources_of(&solution, ActivityId(1));
    let second_resource = resources_of(&solution, ActivityId(2));
    assert_eq!(
        first_resource, second_resource,
        "the goal must keep both occurrences on the same resource"
    );
    assert_eq!(component(&solution, category).value, 0);
}

/// `SpreadActivityOverDays`: occurrences sharing an activity name prefer different period buckets.
#[test]
fn spread_activity_over_days_moves_occurrences_into_different_buckets() {
    let category = "spread over days";
    let day_of = |start: i64| start / 5;

    // Both occurrences pinned inside day 0 collide in the same bucket.
    let clustered = SchedulingProblem::new(
        vec![],
        vec![],
        vec![
            Activity::new(ActivityId(1), "math", TimeWindow::new(0, 1), 1),
            Activity::new(ActivityId(2), "math", TimeWindow::new(1, 2), 1),
        ],
    )
    .with_calendar(period(0, 7), two_day_template())
    .with_soft_goal(SoftGoal::new(
        category,
        ScoreLevel::Weak,
        2,
        SoftGoalKind::SpreadActivityOverDays,
    ));
    let clustered = compile(&clustered).unwrap().solve().solution.unwrap();
    let clustered_component = component(&clustered, category);
    assert_eq!(clustered_component.activity, None);
    assert_eq!(
        clustered_component.value, -2,
        "two occurrences in one bucket must cost one collision * weight 2"
    );

    let spread_activities = || {
        vec![
            Activity::new(ActivityId(1), "math", TimeWindow::new(0, 7), 1),
            Activity::new(ActivityId(2), "math", TimeWindow::new(0, 7), 1),
        ]
    };

    let baseline = SchedulingProblem::new(vec![], vec![], spread_activities())
        .with_calendar(period(0, 7), two_day_template());
    let baseline = compile(&baseline).unwrap().solve().solution.unwrap();
    assert_eq!(
        day_of(start_of(&baseline, ActivityId(1))),
        day_of(start_of(&baseline, ActivityId(2))),
        "without the goal both occurrences start at their earliest slot in day 0"
    );

    let problem = SchedulingProblem::new(vec![], vec![], spread_activities())
        .with_calendar(period(0, 7), two_day_template())
        .with_soft_goal(SoftGoal::new(
            category,
            ScoreLevel::Weak,
            1,
            SoftGoalKind::SpreadActivityOverDays,
        ));
    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    assert_ne!(
        day_of(start_of(&solution, ActivityId(1))),
        day_of(start_of(&solution, ActivityId(2))),
        "the goal must spread the two occurrences across days"
    );
    assert_eq!(component(&solution, category).value, 0);
}
