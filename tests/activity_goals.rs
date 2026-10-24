//! Distribution goals over sets of activities (timbra plan/69, package B).

use schedulr::{
    Activity, ActivityGoal, ActivityGoalKind, ActivityId, DayTemplate, ProximityMode,
    ScheduleTemplate, SchedulingProblem, ScoreLevel, SlotTemplate, Solution, TimeWindow, compile,
};

/// Three days of four units: day d is `[4d, 4d + 4)`.
fn three_days() -> ScheduleTemplate {
    let day = |start: i64| {
        (0..4).fold(DayTemplate::new(start), |day, slot| {
            day.with_slot(SlotTemplate::new(format!("{start}-{slot}"), slot, 1))
        })
    };
    ScheduleTemplate::new(12)
        .with_day(day(0))
        .with_day(day(4))
        .with_day(day(8))
}

fn lesson(id: u64, start: i64, end: i64) -> Activity {
    Activity::new(
        ActivityId(id),
        format!("l{id}"),
        TimeWindow::new(start, end),
        1,
    )
}

fn solve(activities: Vec<Activity>, kind: ActivityGoalKind) -> Solution {
    let problem = SchedulingProblem::new(vec![], vec![], activities)
        .with_schedule_template(three_days())
        .with_activity_goal(ActivityGoal::new("goal", ScoreLevel::Medium, 1, kind));
    compile(&problem)
        .expect("compiles")
        .solve()
        .solution
        .expect("solves")
}

fn start_of(solution: &Solution, id: u64) -> i64 {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(id))
        .expect("placed")
        .window
        .start
}

fn ids(range: std::ops::RangeInclusive<u64>) -> Vec<ActivityId> {
    range.map(ActivityId).collect()
}

/// Two lessons, one pinned to day 0: the other avoids day 1 and lands on day 2.
#[test]
fn not_on_adjacent_days() {
    let solution = solve(
        vec![lesson(1, 0, 1), lesson(2, 4, 12)],
        ActivityGoalKind::NotOnAdjacentBuckets {
            activities: ids(1..=2),
        },
    );
    assert!(start_of(&solution, 2) >= 8);
    assert_eq!(solution.score.medium, 0);
}

/// Variety: pinned at offset 0 on day 0, the second (day 1) takes another offset.
#[test]
fn offsets_vary() {
    let solution = solve(
        vec![lesson(1, 0, 1), lesson(2, 4, 8)],
        ActivityGoalKind::OffsetVariety {
            activities: ids(1..=2),
        },
    );
    assert_ne!(start_of(&solution, 2), 4);
    assert_eq!(solution.score.medium, 0);
}

/// Stability: pinned at offset 2 on day 0, the second (day 1) takes offset 2 as well.
#[test]
fn offsets_stay() {
    let solution = solve(
        vec![lesson(1, 2, 3), lesson(2, 4, 8)],
        ActivityGoalKind::OffsetStability {
            activities: ids(1..=2),
        },
    );
    assert_eq!(start_of(&solution, 2), 6);
}

/// At least one of two lessons before the cut-off at offset 2.
#[test]
fn starts_before_the_cut_off() {
    let solution = solve(
        vec![lesson(1, 2, 4), lesson(2, 0, 12)],
        ActivityGoalKind::StartsWithinOffsets {
            activities: ids(1..=2),
            start_offset: 0,
            end_offset: 2,
            min: 1,
            max: 2,
        },
    );
    assert!(start_of(&solution, 2) % 4 < 2);
    assert_eq!(solution.score.medium, 0);
}

/// Avoid adjacency between two groups: the free lesson of group B does not touch group A's lesson.
#[test]
fn groups_keep_apart() {
    let solution = solve(
        vec![lesson(1, 1, 2), lesson(2, 0, 4)],
        ActivityGoalKind::Proximity {
            groups: vec![vec![ActivityId(1)], vec![ActivityId(2)]],
            mode: ProximityMode::AvoidAdjacent,
        },
    );
    let other = start_of(&solution, 2);
    assert!(
        other == 1 || other == 3,
        "lesson at {other} touches the pinned one at 1 — without participants, sharing its unit is no touch"
    );
}

/// Prefer adjacency: on the same day, the free lesson sits right next to the pinned one.
#[test]
fn groups_come_together() {
    let solution = solve(
        vec![lesson(1, 1, 2), lesson(2, 0, 4)],
        ActivityGoalKind::Proximity {
            groups: vec![vec![ActivityId(1)], vec![ActivityId(2)]],
            mode: ProximityMode::PreferAdjacent,
        },
    );
    let other = start_of(&solution, 2);
    assert!(other == 0 || other == 2, "lesson at {other}");
}

/// A block of two touching lessons inside one day sits at its start or its end.
#[test]
fn long_blocks_sit_at_an_edge() {
    let solution = solve(
        vec![lesson(1, 1, 2), lesson(2, 2, 3), lesson(3, 0, 4)],
        ActivityGoalKind::BlockAtEdges {
            activities: ids(1..=3),
            min_length: 2,
            edges: vec![],
        },
    );
    // Lessons 1 and 2 form a block [1, 3); the third must extend it to an edge: 0 or 3.
    let third = start_of(&solution, 3);
    assert!(third == 0 || third == 3, "third lesson at {third}");
    assert_eq!(solution.score.medium, 0);
}
