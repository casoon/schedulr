//! `IdleBounds`: one entity's free time between its assignments held inside a band, plus a cost
//! per long gap (timbra plan/69, package A).

use schedulr::{
    Activity, ActivityId, EntityRef, IdleBounds, Participant, ParticipantId, SchedulingProblem,
    ScoreLevel, Solution, TimeWindow, compile,
};

const PERSON: ParticipantId = ParticipantId(1);

fn lesson(id: u64, start: i64, end: i64) -> Activity {
    Activity::new(
        ActivityId(id),
        format!("l{id}"),
        TimeWindow::new(start, end),
        1,
    )
    .with_participant(PERSON)
}

fn solve(activities: Vec<Activity>, rule: IdleBounds) -> Solution {
    let problem = SchedulingProblem::new(vec![], vec![Participant::new(PERSON, "P")], activities)
        .with_idle_bounds(rule);
    compile(&problem)
        .expect("compiles")
        .solve()
        .solution
        .expect("solves")
}

fn starts(solution: &Solution) -> Vec<i64> {
    let mut starts: Vec<i64> = solution
        .assignments
        .iter()
        .map(|assignment| assignment.window.start)
        .collect();
    starts.sort_unstable();
    starts
}

fn free_time(starts: &[i64]) -> i64 {
    starts.windows(2).map(|pair| pair[1] - pair[0] - 1).sum()
}

/// A band of exactly one free unit: the solver leaves one gap, neither none nor more.
#[test]
fn free_time_lands_inside_the_band() {
    let solution = solve(
        (1..=3).map(|id| lesson(id, 0, 6)).collect(),
        IdleBounds::new(
            "free periods",
            ScoreLevel::Medium,
            EntityRef::Participant(PERSON),
            1,
            1,
            1,
        ),
    );
    assert_eq!(free_time(&starts(&solution)), 1, "{:?}", starts(&solution));
    assert_eq!(solution.score.medium, 0);
}

/// Two fixed lessons three units apart leave a gap of two; the free lesson splits it, because a
/// long gap costs although the total free time stays the same.
#[test]
fn a_long_gap_is_split() {
    let solution = solve(
        vec![lesson(1, 0, 1), lesson(2, 3, 4), lesson(3, 0, 6)],
        IdleBounds::new(
            "free periods",
            ScoreLevel::Medium,
            EntityRef::Participant(PERSON),
            0,
            10,
            1,
        )
        .with_long_gaps(2, 5),
    );
    let free = starts(&solution)
        .into_iter()
        .find(|&start| start != 0 && start != 3)
        .expect("the free lesson");
    assert!(
        free == 1 || free == 2,
        "the free lesson at {free} leaves a long gap"
    );
    assert_eq!(solution.score.medium, 0);
}
