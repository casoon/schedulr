//! `MaximumBucketStarts`: how many of a group of activities may start inside one bucket — "at most
//! one lesson of this course per day" (timbra plan/62, G5). A hard rule, counted in starts, not
//! in occupied time.

use schedulr::{
    Activity, ActivityId, DayTemplate, MaximumBucketStarts, ScheduleTemplate, SchedulingProblem,
    SlotTemplate, SolveStatus, TimeWindow, compile,
};

/// Three days of four units each: day 0 is `[0, 4)`, day 1 `[4, 8)`, day 2 `[8, 12)`.
fn three_days() -> ScheduleTemplate {
    let mut template = ScheduleTemplate::new(12);
    for day in 0..3 {
        let mut entry = DayTemplate::new(day * 4);
        for slot in 0..4 {
            entry = entry.with_slot(SlotTemplate::new(
                format!("d{day}s{slot}"),
                day * 4 + slot,
                1,
            ));
        }
        template = template.with_day(entry);
    }
    template
}

fn lessons(count: u64, duration: u64, limit: Option<u64>) -> SchedulingProblem {
    let activities: Vec<Activity> = (1..=count)
        .map(|id| {
            Activity::new(
                ActivityId(id),
                format!("l{id}"),
                TimeWindow::new(0, 12),
                duration,
            )
        })
        .collect();
    let mut problem =
        SchedulingProblem::new(vec![], vec![], activities).with_schedule_template(three_days());
    if let Some(limit) = limit {
        problem = problem.with_maximum_bucket_starts(MaximumBucketStarts::new(
            (1..=count).map(ActivityId).collect(),
            limit,
        ));
    }
    problem
}

fn days_used(problem: &SchedulingProblem) -> Vec<i64> {
    let solution = compile(problem)
        .expect("compiles")
        .solve()
        .solution
        .expect("a schedule exists");
    let mut days: Vec<i64> = solution
        .assignments
        .iter()
        .map(|assignment| assignment.window.start / 4)
        .collect();
    days.sort_unstable();
    days
}

/// Three lessons, at most one per day: every day gets exactly one.
#[test]
fn at_most_one_per_day_spreads_the_lessons() {
    assert_eq!(days_used(&lessons(3, 1, Some(1))), vec![0, 1, 2]);
}

/// Four lessons cannot fit three days one at a time — the rule is hard, not a preference, and the
/// search proves it rather than returning a schedule that breaks it.
#[test]
fn the_rule_is_hard() {
    let status = compile(&lessons(4, 1, Some(1)))
        .expect("compiles")
        .solve()
        .status;
    assert_eq!(status, SolveStatus::Infeasible);
    let status = compile(&lessons(4, 1, None))
        .expect("compiles")
        .solve()
        .status;
    assert_eq!(status, SolveStatus::Feasible, "without the rule they fit");
}

/// A double lesson counts once: two doubles and a single fit three days at one start each, which a
/// cap on occupied time (`MaximumDailyLoad`) could not express.
#[test]
fn a_block_counts_once_not_by_its_length() {
    let mut problem = lessons(2, 2, None);
    problem.activities.push(Activity::new(
        ActivityId(3),
        "single",
        TimeWindow::new(0, 12),
        1,
    ));
    problem = problem.with_maximum_bucket_starts(MaximumBucketStarts::new(
        vec![ActivityId(1), ActivityId(2), ActivityId(3)],
        1,
    ));
    assert_eq!(days_used(&problem), vec![0, 1, 2]);
}
