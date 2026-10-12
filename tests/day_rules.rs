//! Day rules and day goals (timbra plan/69, package A): the shape of one entity's occupancy inside
//! each period bucket, hard or as a preference.

use schedulr::{
    Activity, ActivityId, ConcurrentFreeGoal, DayRule, DayRuleKind, DayTemplate, EntityRef,
    IsolatedBlockGoal, JointPresenceGoal, MaximumDistinctPerBucket, Participant, ParticipantId,
    ScheduleTemplate, SchedulingProblem, ScoreLevel, SlotTemplate, SoftPenalty, Solution,
    SolveStatus, TimeWindow, compile,
};

const P: ParticipantId = ParticipantId(1);
const Q: ParticipantId = ParticipantId(2);

/// Two days of six units: day 0 is `[0, 6)`, day 1 is `[6, 12)`.
fn two_days() -> ScheduleTemplate {
    let day = |start: i64| {
        (0..6).fold(DayTemplate::new(start), |day, slot| {
            day.with_slot(SlotTemplate::new(format!("{start}-{slot}"), slot, 1))
        })
    };
    ScheduleTemplate::new(12).with_day(day(0)).with_day(day(6))
}

fn lesson(id: u64, start: i64, end: i64, people: &[ParticipantId]) -> Activity {
    people.iter().fold(
        Activity::new(
            ActivityId(id),
            format!("l{id}"),
            TimeWindow::new(start, end),
            1,
        ),
        |activity, &person| activity.with_participant(person),
    )
}

fn problem(activities: Vec<Activity>) -> SchedulingProblem {
    SchedulingProblem::new(
        vec![],
        vec![Participant::new(P, "P"), Participant::new(Q, "Q")],
        activities,
    )
    .with_schedule_template(two_days())
}

fn solve(problem: &SchedulingProblem) -> (SolveStatus, Option<Solution>) {
    let result = compile(problem).expect("compiles").solve();
    (result.status, result.solution)
}

fn starts(solution: &Solution) -> Vec<i64> {
    let mut starts: Vec<i64> = solution
        .assignments
        .iter()
        .map(|a| a.window.start)
        .collect();
    starts.sort_unstable();
    starts
}

fn day(start: i64) -> i64 {
    start / 6
}

/// Minimum load: three lessons and at least two a day — they cannot spread 1 + 1 + 1 or 2 + 1,
/// so all three share a day.
#[test]
fn minimum_load_keeps_lessons_together() {
    let p =
        problem((1..=3).map(|id| lesson(id, 0, 12, &[P])).collect()).with_day_rule(DayRule::new(
            EntityRef::Participant(P),
            DayRuleKind::MinimumLoad { limit: 2 },
        ));
    let (status, solution) = solve(&p);
    assert_eq!(status, SolveStatus::Feasible);
    let days: Vec<i64> = starts(&solution.unwrap()).into_iter().map(day).collect();
    assert!(days.iter().all(|&d| d == days[0]), "{days:?}");
}

/// Span as a rule: two lessons pinned at 0 and 5 make a span of 6; a limit of 4 is infeasible,
/// as a preference it costs the two units over.
#[test]
fn span_is_hard_or_charged() {
    let base = || problem(vec![lesson(1, 0, 1, &[P]), lesson(2, 5, 6, &[P])]);
    let rule = DayRule::new(
        EntityRef::Participant(P),
        DayRuleKind::MaximumSpan { limit: 4 },
    );
    assert_eq!(
        solve(&base().with_day_rule(rule.clone())).0,
        SolveStatus::Infeasible
    );
    let soft =
        base().with_day_rule(rule.soft(SoftPenalty::new("Verweildauer", ScoreLevel::Weak, 3)));
    let solution = solve(&soft).1.expect("a preference never blocks");
    assert_eq!(solution.score.weak, -6);
}

/// A lunch window `[2, 4)` with a free run of two: the two free lessons of day 0 stay out of it.
#[test]
fn a_free_run_stays_free() {
    let p = problem(vec![
        lesson(1, 0, 6, &[P]),
        lesson(2, 0, 6, &[P]),
        lesson(3, 0, 6, &[P]),
        lesson(4, 0, 6, &[P]),
    ])
    .with_day_rule(DayRule::new(
        EntityRef::Participant(P),
        DayRuleKind::FreeRunWithin {
            start_offset: 2,
            end_offset: 4,
            min_run: 2,
        },
    ));
    let solution = solve(&p).1.expect("solves");
    assert_eq!(starts(&solution), vec![0, 1, 4, 5]);
}

/// Two groups that may not meet on one day: with at most one distinct group a day, the lessons of
/// group A and group B land on different days.
#[test]
fn distinct_groups_are_capped() {
    let p = problem(vec![lesson(1, 0, 12, &[P]), lesson(2, 0, 12, &[P])]).with_distinct_per_bucket(
        MaximumDistinctPerBucket::new(vec![vec![ActivityId(1)], vec![ActivityId(2)]], 1),
    );
    let solution = solve(&p).1.expect("solves");
    let s = starts(&solution);
    assert_ne!(day(s[0]), day(s[1]));
}

/// The isolated-block goal moves a lone afternoon lesson next to the others' half.
#[test]
fn a_lone_half_day_costs() {
    let p = problem(vec![
        lesson(1, 0, 1, &[P]),
        lesson(2, 1, 2, &[P]),
        lesson(3, 0, 6, &[P]),
    ])
    .with_isolated_block_goal(IsolatedBlockGoal {
        category: "Einzelstunde".to_string(),
        level: ScoreLevel::Medium,
        weight: 1,
        entity: EntityRef::Participant(P),
        split_offset: 3,
        max_units: 1,
    });
    let solution = solve(&p).1.expect("solves");
    let free = starts(&solution).into_iter().find(|&s| s >= 2).unwrap_or(0);
    assert!(
        free < 3,
        "the third lesson at {free} sits alone in the afternoon"
    );
    assert_eq!(solution.score.medium, 0);
}

/// Joint presence: P is on two days; on each, one of the lessons it shares with Q must lie.
#[test]
fn the_companion_is_met_every_active_day() {
    let p = problem(vec![
        lesson(1, 0, 1, &[P]),
        lesson(2, 6, 7, &[P]),
        lesson(3, 0, 12, &[P, Q]),
        lesson(4, 0, 12, &[P, Q]),
    ])
    .with_joint_presence_goal(JointPresenceGoal {
        category: "Klassenlehrer".to_string(),
        level: ScoreLevel::Medium,
        weight: 1,
        anchor: EntityRef::Participant(P),
        companion: EntityRef::Participant(Q),
    });
    let solution = solve(&p).1.expect("solves");
    assert_eq!(solution.score.medium, 0);
}

/// Concurrent free: P and Q both teach on day 0 and must not both be free in `[2, 3)`.
#[test]
fn not_everyone_is_free_at_once() {
    let p = problem(vec![
        lesson(1, 0, 1, &[P]),
        lesson(2, 0, 1, &[Q]),
        lesson(3, 0, 6, &[P]),
        lesson(4, 4, 5, &[Q]),
    ])
    .with_concurrent_free_goal(ConcurrentFreeGoal {
        category: "Mittag".to_string(),
        level: ScoreLevel::Medium,
        weight: 1,
        entities: vec![EntityRef::Participant(P), EntityRef::Participant(Q)],
        start_offset: 2,
        end_offset: 3,
        limit: 1,
    });
    let solution = solve(&p).1.expect("solves");
    assert_eq!(solution.score.medium, 0);
}
