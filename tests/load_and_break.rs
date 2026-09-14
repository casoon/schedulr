//! Integration tests for the MaximumDailyLoad and MinimumBreak hard constraints (plan 25, unit A5).
//!
//! Exercises the generic engine primitives through the domain-agnostic `schedulr` API: resources,
//! participants, activities and period buckets derived from a [`ScheduleTemplate`].

use schedulr::{
    AcademicPeriod, Activity, ActivityId, DayTemplate, MaximumDailyLoad, MinimumBreak, Participant,
    ParticipantId, ParticipantPool, ParticipantPoolId, ParticipantRequirement, Resource,
    ResourceId, ResourceRequirement, ScheduleTemplate, SchedulingProblem, SlotTemplate,
    SolveStatus, TimeWindow, compile,
};

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

/// Exactly at the per-bucket limit is allowed; one unit more is infeasible.
#[test]
fn daily_load_at_the_limit_is_allowed_but_one_over_is_infeasible() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    // No calendar: the whole modeled horizon `[0, 2)` is a single bucket.
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 2), 1)
        .with_participant(teacher.id());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 2), 1)
        .with_participant(teacher.id());

    let solve = |limit: u64| {
        let problem = SchedulingProblem::new(
            vec![],
            vec![teacher.clone()],
            vec![first.clone(), second.clone()],
        )
        .with_maximum_daily_load(MaximumDailyLoad::for_participant(teacher.id(), limit));
        compile(&problem).unwrap().solve().status
    };

    assert_eq!(
        solve(2),
        SolveStatus::Feasible,
        "two unit-length lessons exactly at the limit of 2 must fit"
    );
    assert_eq!(
        solve(1),
        SolveStatus::Infeasible,
        "the same two lessons exceed a limit of 1 by one unit"
    );
}

/// Above the limit, the solver spreads the assignments across the day buckets.
#[test]
fn daily_load_moves_an_activity_to_another_day_bucket() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 7), 1)
        .with_participant(teacher.id());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 7), 1)
        .with_participant(teacher.id());
    let problem = SchedulingProblem::new(vec![], vec![teacher], vec![first, second])
        .with_calendar(period(0, 7), two_day_template())
        .with_maximum_daily_load(MaximumDailyLoad::for_participant(ParticipantId(1), 1));

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let starts: Vec<i64> = solution
        .assignments
        .iter()
        .map(|assignment| assignment.window.start)
        .collect();
    let day_of = |start: i64| if start < 5 { 0 } else { 1 };
    assert_ne!(
        day_of(starts[0]),
        day_of(starts[1]),
        "a per-day limit of 1 forces the two unit lessons onto different days: {starts:?}"
    );
}

/// Half-open bucket boundaries: value 4 is the last slot of day 0, value 5 the first of day 1.
#[test]
fn daily_load_respects_day_bucket_boundaries() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let pinned = Activity::new(ActivityId(1), "pinned", TimeWindow::new(4, 5), 1)
        .with_participant(teacher.id());
    let free = Activity::new(ActivityId(2), "free", TimeWindow::new(0, 7), 1)
        .with_participant(teacher.id());
    let problem = SchedulingProblem::new(vec![], vec![teacher], vec![pinned, free])
        .with_calendar(period(0, 7), two_day_template())
        .with_maximum_daily_load(MaximumDailyLoad::for_participant(ParticipantId(1), 1));

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let start_of = |activity: ActivityId| {
        solution
            .assignments
            .iter()
            .find(|assignment| assignment.activity == activity)
            .unwrap()
            .window
            .start
    };
    assert_eq!(start_of(ActivityId(1)), 4);
    assert!(
        start_of(ActivityId(2)) >= 5,
        "start 4 fills day 0, so the free lesson must move to day 1 (start >= 5)"
    );
}

/// Buckets repeat every cycle: value 14 falls into the same bucket as value 4.
#[test]
fn daily_load_buckets_repeat_every_cycle() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let pinned = Activity::new(ActivityId(1), "pinned", TimeWindow::new(14, 15), 1)
        .with_participant(teacher.id());
    let free = Activity::new(ActivityId(2), "free", TimeWindow::new(0, 20), 1)
        .with_participant(teacher.id());
    let problem = SchedulingProblem::new(vec![], vec![teacher], vec![pinned, free])
        .with_calendar(period(0, 20), two_day_template())
        .with_maximum_daily_load(MaximumDailyLoad::for_participant(ParticipantId(1), 1));

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let free_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    // Residual 4 (value 14) belongs to day 0 again, so the free lesson must land in day 1.
    assert!(
        free_start.rem_euclid(10) >= 5,
        "value 14 shares day 0's bucket with value 4, forcing the free lesson to day 1: {free_start}"
    );
}

/// An activity shared by several entities is counted for each of them separately; resources and
/// participants are tracked independently.
#[test]
fn daily_load_counts_each_participant_and_resource_separately() {
    let first_teacher = Participant::new(ParticipantId(1), "Teacher A");
    let second_teacher = Participant::new(ParticipantId(2), "Teacher B");
    let room = Resource::new(ResourceId(1), "Room", 1);

    let joint = Activity::new(ActivityId(1), "joint", TimeWindow::new(0, 2), 1)
        .with_participant(first_teacher.id())
        .with_participant(second_teacher.id())
        .with_requirement(ResourceRequirement::new(room.id(), 1));
    let solo = Activity::new(ActivityId(2), "solo", TimeWindow::new(0, 2), 1)
        .with_participant(first_teacher.id())
        .with_requirement(ResourceRequirement::new(room.id(), 1));

    let solve = |loads: Vec<MaximumDailyLoad>| {
        let mut problem = SchedulingProblem::new(
            vec![room.clone()],
            vec![first_teacher.clone(), second_teacher.clone()],
            vec![joint.clone(), solo.clone()],
        );
        for load in loads {
            problem = problem.with_maximum_daily_load(load);
        }
        compile(&problem).unwrap().solve().status
    };

    // Teacher A attends both activities: 2 units.
    assert_eq!(
        solve(vec![MaximumDailyLoad::for_participant(ParticipantId(1), 2)]),
        SolveStatus::Feasible
    );
    assert_eq!(
        solve(vec![MaximumDailyLoad::for_participant(ParticipantId(1), 1)]),
        SolveStatus::Infeasible,
        "teacher A accumulates both lessons"
    );
    // Teacher B attends only the joint activity: 1 unit.
    assert_eq!(
        solve(vec![MaximumDailyLoad::for_participant(ParticipantId(2), 1)]),
        SolveStatus::Feasible
    );
    // The room is occupied by both activities (back-to-back): 2 units.
    assert_eq!(
        solve(vec![MaximumDailyLoad::for_resource(ResourceId(1), 1)]),
        SolveStatus::Infeasible,
        "the room accumulates both activities separately from any participant"
    );
}

/// A minimum break exactly satisfied is allowed; a too-short one is rejected.
#[test]
fn minimum_break_is_satisfied_exactly_and_rejected_when_too_short() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let pinned = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1)
        .with_participant(teacher.id());

    let solve = |window: TimeWindow, min_distance: i64| {
        let free = Activity::new(ActivityId(2), "second", window, 1).with_participant(teacher.id());
        let problem =
            SchedulingProblem::new(vec![], vec![teacher.clone()], vec![pinned.clone(), free])
                .with_minimum_break(MinimumBreak::for_participant(teacher.id(), min_distance));
        compile(&problem).unwrap().solve()
    };

    // The minimum distance of 2 is exactly met by starting at 2.
    let result = solve(TimeWindow::new(0, 4), 2);
    let solution = result
        .solution
        .expect("feasible at exactly the minimum distance");
    let free_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    assert_eq!(
        free_start, 2,
        "the solver takes the earliest start meeting the distance"
    );

    // The only candidate start (1) is closer than the minimum -> infeasible.
    assert_eq!(
        solve(TimeWindow::new(1, 2), 2).status,
        SolveStatus::Infeasible
    );
}

/// `MinimumBreak` is an independent concept: it constrains the distance between two assignments
/// without any `BreakTemplate`, and only takes effect when it is actually added.
#[test]
fn minimum_break_is_an_independent_rule() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1)
        .with_participant(teacher.id());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(2, 3), 1)
        .with_participant(teacher.id());

    // Two lessons two units apart: no minimum-break rule, so this is fine.
    let without_break = SchedulingProblem::new(
        vec![],
        vec![teacher.clone()],
        vec![first.clone(), second.clone()],
    );
    assert_eq!(
        compile(&without_break).unwrap().solve().status,
        SolveStatus::Feasible
    );

    // A minimum break of 3 leaves the second lesson without a valid start -> infeasible.
    let with_break = SchedulingProblem::new(vec![], vec![teacher], vec![first, second])
        .with_minimum_break(MinimumBreak::for_participant(ParticipantId(1), 3));
    assert_eq!(
        compile(&with_break).unwrap().solve().status,
        SolveStatus::Infeasible
    );
}

/// An entity chosen out of a candidate pool counts toward the bucket load exactly like a fixed
/// assignment: with a per-bucket cap of 1 on participant A, the pool-selected lesson A would
/// otherwise take must fall to B.
#[test]
fn daily_load_counts_a_participant_chosen_from_a_candidate_pool() {
    let first_teacher = Participant::new(ParticipantId(1), "Teacher A");
    // B cannot attend the first lesson's window, so that lesson's pool choice is forced onto A.
    let second_teacher =
        Participant::new(ParticipantId(2), "Teacher B").with_unavailable_range(0, 2);
    let pool = ParticipantPool::new(
        ParticipantPoolId(1),
        "Teachers",
        [first_teacher.id(), second_teacher.id()],
    );
    // Both lessons need exactly one participant chosen out of {A, B}.
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 2), 1)
        .with_participant_requirement(ParticipantRequirement::from_pool(pool.id));
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(2, 4), 1)
        .with_participant_requirement(ParticipantRequirement::from_pool(pool.id));

    let solve = |limit: Option<u64>| {
        let mut problem = SchedulingProblem::new(
            vec![],
            vec![first_teacher.clone(), second_teacher.clone()],
            vec![first.clone(), second.clone()],
        )
        .with_participant_pool(pool.clone());
        if let Some(limit) = limit {
            problem = problem.with_maximum_daily_load(MaximumDailyLoad::for_participant(
                first_teacher.id(),
                limit,
            ));
        }
        let solution = compile(&problem)
            .unwrap()
            .solve()
            .solution
            .expect("both lessons fit");
        let selected = |activity: ActivityId| {
            solution
                .assignments
                .iter()
                .find(|assignment| assignment.activity == activity)
                .unwrap()
                .participants
                .clone()
        };
        (selected(ActivityId(1)), selected(ActivityId(2)))
    };

    // Without the cap the first lesson lands on A (B is unavailable) and the second defaults to A,
    // so A would be assigned to both lessons.
    assert_eq!(
        solve(None),
        (vec![ParticipantId(1)], vec![ParticipantId(1)]),
        "baseline: the pool selection defaults to A for both lessons"
    );

    // The cap of 1 on A must move the second lesson's pool selection to B.
    assert_eq!(
        solve(Some(1)),
        (vec![ParticipantId(1)], vec![ParticipantId(2)]),
        "the load cap must force one pool selection to B"
    );
}

/// A participant bound through a single-candidate requirement is still counted: the requirement's
/// exactly-one forces the selection, so two lessons served that way exceed a per-bucket cap of 1.
#[test]
fn daily_load_counts_a_participant_bound_by_a_single_candidate_requirement() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let requirement = ParticipantRequirement::matching().with_candidate(teacher.id());
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 2), 1)
        .with_participant_requirement(requirement.clone());
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 2), 1)
        .with_participant_requirement(requirement);

    let solve = |limit: u64| {
        let problem = SchedulingProblem::new(
            vec![],
            vec![teacher.clone()],
            vec![first.clone(), second.clone()],
        )
        .with_maximum_daily_load(MaximumDailyLoad::for_participant(teacher.id(), limit));
        compile(&problem).unwrap().solve().status
    };

    assert_eq!(solve(2), SolveStatus::Feasible);
    assert_eq!(
        solve(1),
        SolveStatus::Infeasible,
        "a participant chosen through a requirement must count toward the load cap"
    );
}

/// `MinimumBreak` also pairs a participant bound through a single-candidate requirement, keeping
/// it apart from that participant's fixed assignment.
#[test]
fn minimum_break_pairs_a_participant_bound_by_a_single_candidate_requirement() {
    let teacher = Participant::new(ParticipantId(1), "Teacher");
    let pinned = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1)
        .with_participant(teacher.id());
    let flexible = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 4), 1)
        .with_participant_requirement(
            ParticipantRequirement::matching().with_candidate(teacher.id()),
        );
    let problem = SchedulingProblem::new(vec![], vec![teacher], vec![pinned, flexible])
        .with_minimum_break(MinimumBreak::for_participant(ParticipantId(1), 2));

    let solution = compile(&problem).unwrap().solve().solution.unwrap();
    let second_start = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(2))
        .unwrap()
        .window
        .start;
    assert_eq!(
        second_start, 2,
        "the requirement-bound lesson must be paired with the fixed one and take the earliest start \
         satisfying the break"
    );
}
