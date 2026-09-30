//! `ConsecutiveRunLimit`: a soft cap on back-to-back runs of one group of activities — "no more than
//! two core-subject lessons in a row" (timbra plan/62, G7).

use schedulr::{
    Activity, ActivityId, ConsecutiveRunLimit, DayTemplate, Participant, ParticipantId,
    ScheduleTemplate, SchedulingProblem, ScoreLevel, SlotTemplate, SolveOptions, TimeWindow,
    compile,
};
use std::time::Duration;

/// One day of four units.
fn one_day() -> ScheduleTemplate {
    let mut day = DayTemplate::new(0);
    for slot in 0..4 {
        day = day.with_slot(SlotTemplate::new(format!("s{slot}"), slot, 1));
    }
    ScheduleTemplate::new(4).with_day(day)
}

/// Three core lessons and one other, all for the same class, filling the day.
fn day_of_one_class(limit: bool) -> SchedulingProblem {
    let class = ParticipantId(1);
    let activities: Vec<Activity> = (1..=4)
        .map(|id| {
            Activity::new(ActivityId(id), format!("l{id}"), TimeWindow::new(0, 4), 1)
                .with_participant(class)
        })
        .collect();
    let mut problem =
        SchedulingProblem::new(vec![], vec![Participant::new(class, "7a")], activities)
            .with_schedule_template(one_day());
    if limit {
        problem = problem.with_consecutive_run_limit(ConsecutiveRunLimit::new(
            "Hauptfächer am Stück",
            ScoreLevel::Medium,
            1,
            vec![ActivityId(1), ActivityId(2), ActivityId(3)],
            2,
        ));
    }
    problem
}

/// The other lesson ends up between the core ones: never three core lessons in a row.
#[test]
fn the_other_lesson_breaks_the_run() {
    let compiled = compile(&day_of_one_class(true)).expect("compiles");
    let solution = compiled
        .solve_with(&SolveOptions {
            time_limit: Some(Duration::from_secs(2)),
            ..SolveOptions::default()
        })
        .solution
        .expect("the day fits");
    let other = solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(4))
        .expect("placed")
        .window
        .start;
    assert!(
        other == 1 || other == 2,
        "the other lesson at {other} leaves three core lessons back to back"
    );
    assert_eq!(solution.score.medium, 0, "no run penalty left");
}
