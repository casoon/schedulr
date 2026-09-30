//! `ScoreRuleKind::PreferWithinBucket`: a soft wish about the position *inside* every bucket —
//! "not in the first unit of a day" (timbra plan/62, G3). `PreferWindow` names one window over the
//! whole horizon and cannot say that.

use schedulr::{
    Activity, ActivityId, DayTemplate, ScheduleTemplate, SchedulingProblem, ScoreLevel, ScoreRule,
    SlotTemplate, TimeWindow, compile,
};

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

fn starts(window: TimeWindow, count: u64) -> Vec<i64> {
    let activities: Vec<Activity> = (1..=count)
        .map(|id| Activity::new(ActivityId(id), format!("l{id}"), window, 1))
        .collect();
    let mut problem =
        SchedulingProblem::new(vec![], vec![], activities).with_schedule_template(three_days());
    for id in 1..=count {
        problem = problem.with_score_rule(ScoreRule::prefer_within_bucket(
            "nicht zuerst",
            ScoreLevel::Medium,
            ActivityId(id),
            1,
            4,
            1,
        ));
    }
    let solution = compile(&problem)
        .expect("compiles")
        .solve()
        .solution
        .expect("a schedule exists");
    solution
        .assignments
        .iter()
        .map(|assignment| assignment.window.start)
        .collect()
}

/// Free to go anywhere, the lessons avoid the first unit of every day.
#[test]
fn the_wish_keeps_lessons_out_of_the_first_unit() {
    for start in starts(TimeWindow::new(0, 12), 3) {
        assert_ne!(start % 4, 0, "start {start} is the first unit of its day");
    }
}

/// It is a wish, not a rule: a lesson that can only take the first unit still gets it.
#[test]
fn the_wish_never_makes_a_schedule_impossible() {
    assert_eq!(starts(TimeWindow::new(4, 5), 1), vec![4]);
}
