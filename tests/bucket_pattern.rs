use schedulr::{
    Activity, ActivityId, BucketLoadPattern, DayTemplate, ScheduleTemplate, SchedulingProblem,
    SlotTemplate, SolveStatus, TimeWindow, bucket_load_blocks, bucket_load_pattern, bucket_windows,
    compile, matches_bucket_load_pattern,
};

fn window(start: i64, end: i64) -> TimeWindow {
    TimeWindow::new(start, end)
}

/// C1: adjacent slots are **one** block — a "double period" is only observable this way.
#[test]
fn touching_slots_form_one_block() {
    let buckets = [window(0, 8)];
    let assignments = [
        (ActivityId(1), window(0, 1)),
        (ActivityId(2), window(1, 2)),
        (ActivityId(3), window(5, 6)),
    ];

    assert_eq!(bucket_load_blocks(&assignments, &buckets), vec![vec![2, 1]]);
    assert_eq!(bucket_load_pattern(&assignments, &buckets), vec![2, 1]);
}

/// C1: a gap separates blocks, and a bucket boundary is a real break — a block never spans two
/// buckets (the boundary is whatever the caller's schedule template says, e.g. the end of a day).
#[test]
fn gaps_and_bucket_boundaries_separate_blocks() {
    let buckets = [window(0, 4), window(4, 8)];
    let assignments = [(ActivityId(1), window(3, 6))];

    assert_eq!(
        bucket_load_blocks(&assignments, &buckets),
        vec![vec![1], vec![2]]
    );
    assert_eq!(bucket_load_pattern(&assignments, &buckets), vec![2, 1]);
}

/// C1: the order inside an allowed pattern is irrelevant — `[2, 1, 1]` and `[1, 2, 1]` are the
/// same pattern — while a genuinely different shape is rejected.
#[test]
fn pattern_order_does_not_matter_but_the_shape_does() {
    let buckets = [window(0, 8), window(8, 16)];
    let course = vec![ActivityId(1), ActivityId(2), ActivityId(3), ActivityId(4)];
    // Monday: double + single, Tuesday: single → [2, 1, 1].
    let plan = [
        (ActivityId(1), window(0, 1)),
        (ActivityId(2), window(1, 2)),
        (ActivityId(3), window(3, 4)),
        (ActivityId(4), window(9, 10)),
    ];

    let as_written = BucketLoadPattern::new(course.clone(), vec![vec![1, 2, 1]]);
    assert!(matches_bucket_load_pattern(&as_written, &plan, &buckets));

    let singles_only = BucketLoadPattern::new(course, vec![vec![1, 1, 1, 1]]);
    assert!(!matches_bucket_load_pattern(&singles_only, &plan, &buckets));
}

/// C1: "no pattern chosen yet" must not turn into a violation — and the course then also does
/// not get "anything goes" enforced, it simply has no rule.
#[test]
fn an_inactive_pattern_is_never_violated() {
    let buckets = [window(0, 8)];
    let plan = [(ActivityId(1), window(0, 1))];

    let no_activities = BucketLoadPattern::new(Vec::new(), vec![vec![1]]);
    let no_allowed = BucketLoadPattern::new(vec![ActivityId(1)], Vec::new());

    assert!(!no_activities.is_active());
    assert!(!no_allowed.is_active());
    assert!(matches_bucket_load_pattern(&no_activities, &plan, &buckets));
    assert!(matches_bucket_load_pattern(&no_allowed, &plan, &buckets));
}

/// C1: buckets come from the schedule template — one per day, and the same day of the cycle keeps
/// its bucket index in the next cycle — while a problem without a template has one bucket.
#[test]
fn buckets_come_from_the_schedule_template() {
    let template = ScheduleTemplate::new(8)
        .with_day(DayTemplate::new(0))
        .with_day(DayTemplate::new(3));

    let windows = bucket_windows(Some(&template), 0, 8);
    let covering: Vec<(usize, i64, i64)> = windows
        .iter()
        .filter(|entry| entry.window.start >= 0 && entry.window.start < 8)
        .map(|entry| (entry.bucket, entry.window.start, entry.window.end))
        .collect();
    assert_eq!(covering, vec![(0, 0, 3), (1, 3, 8)]);

    // Cycle two repeats the bucket indices instead of inventing new ones.
    let next_cycle = windows
        .iter()
        .find(|entry| entry.window.start == 8)
        .expect("a window for the next cycle");
    assert_eq!((next_cycle.bucket, next_cycle.window.end), (0, 11));

    // No template: the whole horizon is one bucket.
    let plain = bucket_windows(None, 0, 20);
    assert_eq!(plain.len(), 1);
    assert_eq!(
        (plain[0].bucket, plain[0].window.start, plain[0].window.end),
        (0, 0, 20)
    );
}

/// A two-day template over a cycle of 8: day 0 is `[0, 4)`, day 1 is `[4, 8)`.
fn two_day_template() -> ScheduleTemplate {
    ScheduleTemplate::new(8)
        .with_day(DayTemplate::new(0).with_slot(SlotTemplate::new("d0", 0, 1)))
        .with_day(DayTemplate::new(4).with_slot(SlotTemplate::new("d1", 4, 1)))
}

/// One course of two single periods, each pinned to its own day: the first can only occupy day 0,
/// the second only day 1.
fn two_singles_on_separate_days(allowed: Vec<Vec<u64>>) -> SchedulingProblem {
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1);
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(4, 5), 1);
    SchedulingProblem::new(vec![], vec![], vec![first, second])
        .with_schedule_template(two_day_template())
        .with_bucket_load_pattern(BucketLoadPattern::new(
            vec![ActivityId(1), ActivityId(2)],
            allowed,
        ))
}

/// C1: the pattern is a **hard constraint**, not advice. Two single periods pinned to different
/// days can never be the `[2]` block of a double period, so the identical input is feasible under
/// `[1, 1]` and infeasible under `[2]`. Without the compiled rule both would solve, and a course's
/// chosen teaching shape would be a suggestion rather than a rule.
#[test]
fn the_pattern_is_a_hard_constraint() {
    let feasible = compile(&two_singles_on_separate_days(vec![vec![1, 1]]))
        .expect("the problem compiles")
        .solve()
        .status;
    assert_eq!(
        feasible,
        SolveStatus::Feasible,
        "two single blocks on two days match [1, 1]"
    );

    let infeasible = compile(&two_singles_on_separate_days(vec![vec![2]]))
        .expect("the problem compiles")
        .solve()
        .status;
    assert_eq!(
        infeasible,
        SolveStatus::Infeasible,
        "the same two periods can never form the single block of two that [2] demands"
    );
}

/// A pattern without an allowed shape (or without activities) is not compiled at all: the problem
/// stays feasible, so "no pattern chosen yet" can never reject a plan.
#[test]
fn an_inactive_pattern_is_not_compiled() {
    let status = compile(&two_singles_on_separate_days(Vec::new()))
        .expect("the problem compiles")
        .solve()
        .status;
    assert_eq!(status, SolveStatus::Feasible);
}

/// The rule really shapes the plan: with both periods free inside one day and `[2]` allowed, the
/// solver has to put them next to each other — nothing else in the problem forces that.
#[test]
fn the_rule_places_two_periods_adjacent() {
    let template = ScheduleTemplate::new(4).with_day(
        DayTemplate::new(0)
            .with_slot(SlotTemplate::new("s0", 0, 1))
            .with_slot(SlotTemplate::new("s1", 1, 1))
            .with_slot(SlotTemplate::new("s2", 2, 1))
            .with_slot(SlotTemplate::new("s3", 3, 1)),
    );
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 4), 1);
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 4), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![first, second])
        .with_schedule_template(template)
        .with_bucket_load_pattern(BucketLoadPattern::new(
            vec![ActivityId(1), ActivityId(2)],
            vec![vec![2]],
        ));

    let solution = compile(&problem)
        .expect("the problem compiles")
        .solve()
        .solution
        .expect("a double period fits in the day");
    let start_of = |activity: ActivityId| {
        solution
            .assignments
            .iter()
            .find(|assignment| assignment.activity == activity)
            .expect("every activity is placed")
            .window
            .start
    };
    assert_eq!(
        (start_of(ActivityId(1)) - start_of(ActivityId(2))).abs(),
        1,
        "only adjacent periods form the block of two the pattern requires"
    );
}
