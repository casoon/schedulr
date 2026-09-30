//! `MaximumActiveBuckets`: an entity active on at most k buckets, which ones left to the search —
//! "a part-time teacher has one free day, the plan decides which" (timbra plan/62, G6).

use schedulr::{
    Activity, ActivityId, DayTemplate, MaximumActiveBuckets, Participant, ParticipantId,
    ScheduleTemplate, SchedulingProblem, SlotTemplate, SolveStatus, TimeWindow, compile,
};

/// Three days of four units each.
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

/// `count` lessons of `duration` units for one teacher, free anywhere in the three days.
fn teacher_with(count: u64, duration: u64, days: Option<u64>) -> SchedulingProblem {
    let teacher = ParticipantId(1);
    let activities: Vec<Activity> = (1..=count)
        .map(|id| {
            Activity::new(
                ActivityId(id),
                format!("l{id}"),
                TimeWindow::new(0, 12),
                duration,
            )
            .with_participant(teacher)
        })
        .collect();
    let mut problem = SchedulingProblem::new(
        vec![],
        vec![Participant::new(teacher, "Teilzeit")],
        activities,
    )
    .with_schedule_template(three_days());
    if let Some(days) = days {
        problem = problem
            .with_maximum_active_buckets(MaximumActiveBuckets::for_participant(teacher, days));
    }
    problem
}

/// Six lessons, at most two days: they fit, and they really use only two.
#[test]
fn the_lessons_use_at_most_the_allowed_days() {
    let solution = compile(&teacher_with(6, 1, Some(2)))
        .expect("compiles")
        .solve()
        .solution
        .expect("six units fit into two days of four");
    let mut days: Vec<i64> = solution
        .assignments
        .iter()
        .map(|assignment| assignment.window.start / 4)
        .collect();
    days.sort_unstable();
    days.dedup();
    assert!(days.len() <= 2, "used days {days:?}");
}

/// Five double lessons do not fit two days of four — a hard rule, proven, not broken.
///
/// Doubles on purpose: nine single lessons on eight places is the same argument, but proving it
/// is a pigeonhole over the teacher's timeline, which the unary-resource propagation does not
/// draw — the search then tries arrangements until the budget ends. That is a limit of the
/// resource propagation, not of this rule; the rule never lets a schedule through either way.
#[test]
fn the_rule_is_hard() {
    let status = compile(&teacher_with(5, 2, Some(2)))
        .expect("compiles")
        .solve()
        .status;
    assert_eq!(status, SolveStatus::Infeasible);
    let status = compile(&teacher_with(5, 2, None))
        .expect("compiles")
        .solve()
        .status;
    assert_eq!(status, SolveStatus::Feasible, "without the rule they fit");
}
