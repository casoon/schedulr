//! `TimePreference`: one entity's occupancy measured against valued windows — "rather not in the
//! first unit", "ideally at the end" (timbra plan/69, package A).

use schedulr::{
    Activity, ActivityId, EntityRef, Participant, ParticipantId, SchedulingProblem, ScoreLevel,
    TimePreference, TimeWindow, compile,
};

fn problem(windows: Vec<(TimeWindow, i64)>) -> SchedulingProblem {
    let person = Participant::new(ParticipantId(1), "P");
    let lesson =
        Activity::new(ActivityId(1), "l", TimeWindow::new(0, 4), 1).with_participant(person.id());
    SchedulingProblem::new(vec![], vec![person], vec![lesson]).with_time_preference(
        TimePreference::new(
            "time preference",
            ScoreLevel::Medium,
            EntityRef::Participant(ParticipantId(1)),
            windows,
        ),
    )
}

fn start(problem: &SchedulingProblem) -> (i64, i64) {
    let solution = compile(problem)
        .expect("compiles")
        .solve()
        .solution
        .expect("solves");
    (solution.assignments[0].window.start, solution.score.medium)
}

/// An unwanted window is avoided where there is room.
#[test]
fn an_unwanted_window_is_avoided() {
    let (at, score) = start(&problem(vec![(TimeWindow::new(0, 3), -3)]));
    assert_eq!(at, 3, "the only unit outside the unwanted window");
    assert_eq!(score, 0);
}

/// A wanted window attracts the lesson and its value is credited.
#[test]
fn a_wanted_window_attracts_and_scores() {
    let (at, score) = start(&problem(vec![(TimeWindow::new(2, 3), 2)]));
    assert_eq!(at, 2);
    assert_eq!(score, 2);
}

/// A window that ends before it starts is refused.
#[test]
fn an_inverted_window_is_refused() {
    let broken = problem(vec![(TimeWindow { start: 3, end: 1 }, -1)]);
    assert!(compile(&broken).is_err());
}

/// A preference can name a single activity: its own occupancy is measured.
#[test]
fn an_activity_can_carry_its_own_preference() {
    let lesson = Activity::new(ActivityId(1), "l", TimeWindow::new(0, 4), 1);
    let problem = SchedulingProblem::new(vec![], vec![], vec![lesson]).with_time_preference(
        TimePreference::new(
            "course preference",
            ScoreLevel::Medium,
            EntityRef::Activity(ActivityId(1)),
            vec![(TimeWindow::new(1, 2), 3)],
        ),
    );
    assert_eq!(start(&problem), (1, 3));
}
