//! A blocked window forbids **occupying** it, not just starting inside it (plan 26, §2).
//!
//! `with_unavailable_range` keeps its start semantics — a range that a long activity may start
//! before. `with_blocked_window` is the timetable reading: "this person/room is not bookable in
//! these hours", whatever the lesson's length.

use schedulr::{
    Activity, ActivityId, Assignment, ChangeRequest, CompiledProblem, Participant, ParticipantId,
    ParticipantRequirement, Resource, ResourceId, ResourceRequirement, SchedulingProblem, Score,
    Solution, TimeWindow, compile,
};

/// One lesson of `duration` for Ben, who is not bookable in the 4th hour (`[3, 4)`).
fn lesson_for_ben(duration: u64) -> CompiledProblem {
    let problem = SchedulingProblem::new(
        vec![],
        vec![
            Participant::new(ParticipantId(1), "Ben").with_blocked_window(TimeWindow::new(3, 4)),
            Participant::new(ParticipantId(2), "Cara"),
        ],
        vec![
            Activity::new(ActivityId(1), "Mathematik", TimeWindow::new(0, 6), duration)
                .with_participant(ParticipantId(1)),
        ],
    );
    compile(&problem).expect("a valid problem")
}

/// The same lesson, placed at `start` — the baseline a move is evaluated against.
fn placed_at(start: i64, duration: u64) -> Solution {
    Solution {
        assignments: vec![Assignment::new(
            ActivityId(1),
            TimeWindow::new(start, start + duration as i64),
        )],
        score: Score::default(),
        score_components: Vec::new(),
    }
}

/// Whether moving the lesson to `start` is allowed.
fn may_start_at(duration: u64, start: i64) -> bool {
    lesson_for_ben(duration)
        .evaluate_changes(
            &placed_at(0, duration),
            &[ChangeRequest::Move {
                activity: ActivityId(1),
                window: TimeWindow::new(start, start + duration as i64),
            }],
        )
        .is_feasible
}

#[test]
fn blocked_starts_and_occupies_describe_the_same_slots() {
    let window = TimeWindow::new(3, 4);
    let occupied = |duration: i64| {
        (0..8)
            .filter(|start| window.occupies(*start, duration))
            .collect::<Vec<_>>()
    };

    // A one-hour lesson is pushed off that hour alone …
    assert_eq!(window.blocked_starts(1), 3..=3);
    assert_eq!(occupied(1), vec![3]);
    // … a two-hour lesson also off the hour before it, because it would run into the block.
    assert_eq!(window.blocked_starts(2), 2..=3);
    assert_eq!(occupied(2), vec![2, 3]);
    assert_eq!(window.blocked_starts(3), 1..=3);
    assert_eq!(occupied(3), vec![1, 2, 3]);
    // A window of zero length blocks nothing — half-open, `[3, 3)` is empty.
    assert!(window.blocked_starts(0).is_empty());
    assert!(occupied(0).is_empty());
}

#[test]
fn a_single_lesson_avoids_the_blocked_hour_but_not_its_neighbours() {
    assert!(!may_start_at(1, 3), "die gesperrte Stunde selbst");
    assert!(may_start_at(1, 2), "unmittelbar davor");
    assert!(may_start_at(1, 4), "unmittelbar danach");
}

#[test]
fn a_long_lesson_may_not_reach_into_the_blocked_hour() {
    // Two hours starting at 2 would run through the blocked hour 3 — that is the whole point of
    // the block, and exactly what a start-only range would have let through.
    assert!(!may_start_at(2, 2), "läuft in die gesperrte Stunde hinein");
    assert!(
        may_start_at(2, 1),
        "endet genau mit dem Beginn der Sperre — halboffen erlaubt"
    );
    assert!(may_start_at(2, 4), "beginnt genau am Ende der Sperre");
}

#[test]
fn a_blocked_candidate_is_not_chosen_for_that_slot() {
    // The lesson *must* sit in the blocked hour: with Ben blocked, only Cara is left.
    let problem = SchedulingProblem::new(
        vec![],
        vec![
            Participant::new(ParticipantId(1), "Ben").with_blocked_window(TimeWindow::new(3, 4)),
            Participant::new(ParticipantId(2), "Cara"),
        ],
        vec![
            Activity::new(ActivityId(1), "Mathematik", TimeWindow::new(3, 4), 1)
                .with_participant_requirement(ParticipantRequirement::matching()),
        ],
    );
    let solution = compile(&problem)
        .expect("a valid problem")
        .solve()
        .solution
        .expect("Cara can take the lesson");

    assert_eq!(solution.assignments[0].participants, vec![ParticipantId(2)]);
}

#[test]
fn a_blocked_room_is_avoided_like_a_blocked_person() {
    let problem = SchedulingProblem::new(
        vec![Resource::new(ResourceId(1), "A204", 1).with_blocked_window(TimeWindow::new(3, 4))],
        vec![],
        vec![
            Activity::new(ActivityId(1), "Mathematik", TimeWindow::new(0, 6), 1)
                .with_requirement(ResourceRequirement::new(ResourceId(1), 1)),
        ],
    );
    let compiled = compile(&problem).expect("a valid problem");
    let solution = Solution {
        assignments: vec![
            Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
        ],
        score: Score::default(),
        score_components: Vec::new(),
    };
    let moved = |start: i64| {
        compiled
            .evaluate_changes(
                &solution,
                &[ChangeRequest::Move {
                    activity: ActivityId(1),
                    window: TimeWindow::new(start, start + 1),
                }],
            )
            .is_feasible
    };

    assert!(!moved(3));
    assert!(moved(2));
    assert!(moved(4));
}

#[test]
fn the_start_semantics_of_an_unavailable_range_are_unchanged() {
    // The additive promise: `with_unavailable_range` still blocks starts only, so the existing
    // booking-desk behaviour (a long activity may start before the range) survives.
    let problem = SchedulingProblem::new(
        vec![],
        vec![Participant::new(ParticipantId(1), "Ben").with_unavailable_range(3, 3)],
        vec![
            Activity::new(ActivityId(1), "Besprechung", TimeWindow::new(0, 6), 2)
                .with_participant(ParticipantId(1)),
        ],
    );
    let compiled = compile(&problem).expect("a valid problem");
    let solution = Solution {
        assignments: vec![Assignment::new(ActivityId(1), TimeWindow::new(0, 2))],
        score: Score::default(),
        score_components: Vec::new(),
    };
    let moved = |start: i64| {
        compiled
            .evaluate_changes(
                &solution,
                &[ChangeRequest::Move {
                    activity: ActivityId(1),
                    window: TimeWindow::new(start, start + 2),
                }],
            )
            .is_feasible
    };

    assert!(!moved(3), "der Start liegt in der Sperre");
    assert!(moved(2), "ragt in die Sperre hinein, darf aber starten");
}
