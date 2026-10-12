//! A repair after a new restriction: a solved timetable, then one period is blocked for every
//! group, and the repair has to move what sat there — few moves, no new conflicts.
//!
//! The shape is the one that broke in practice (timbra plan 53, 2026-10-12): every lesson that
//! leaves the blocked period lands on a slot where its group, teacher or room is already busy,
//! so no single move lowers the conflict count. Only chains of moves — the displaced lesson
//! pushes another one on — repair it.

use schedulr::{
    Activity, ActivityId, Participant, ParticipantId, RepairOptions, Resource, ResourceId,
    ResourceRequirement, SchedulingProblem, Solution, SolveOptions, TimeWindow, compile,
};
use std::collections::BTreeMap;
use std::time::Duration;

/// Five days of six periods.
const WEEK: i64 = 30;
const GROUPS: u64 = 8;
const LESSONS_PER_GROUP: u64 = 24;
const TEACHERS: u64 = 12;

fn problem(blocked: Option<i64>) -> SchedulingProblem {
    let resources: Vec<Resource> = (1..=GROUPS)
        .map(|group| Resource::new(ResourceId(group), format!("room{group}"), 1))
        .collect();
    let mut participants: Vec<Participant> = (1..=GROUPS)
        .map(|group| {
            let participant = Participant::new(ParticipantId(group), format!("group{group}"));
            match blocked {
                Some(slot) => participant.with_blocked_window(TimeWindow::new(slot, slot + 1)),
                None => participant,
            }
        })
        .collect();
    participants.extend(
        (1..=TEACHERS)
            .map(|teacher| Participant::new(ParticipantId(100 + teacher), format!("t{teacher}"))),
    );
    let mut activities = Vec::new();
    for group in 1..=GROUPS {
        for index in 0..LESSONS_PER_GROUP {
            let id = ActivityId(group * 100 + index);
            // Two teachers per group share its lessons; neighbouring groups share a teacher, so
            // the groups are coupled the way a real staff couples classes.
            let teacher = 100 + ((group * 3 + index % 2) % TEACHERS) + 1;
            activities.push(
                Activity::new(id, format!("g{group}l{index}"), TimeWindow::new(0, WEEK), 1)
                    .with_requirement(ResourceRequirement::new(ResourceId(group), 1))
                    .with_participant(ParticipantId(group))
                    .with_participant(ParticipantId(teacher)),
            );
        }
    }
    SchedulingProblem::new(resources, participants, activities)
}

fn starts(solution: &Solution) -> BTreeMap<ActivityId, i64> {
    solution
        .assignments
        .iter()
        .map(|assignment| (assignment.activity, assignment.window.start))
        .collect()
}

#[test]
fn a_new_block_is_repaired_by_moving_few_lessons() {
    let free = compile(&problem(None)).expect("compiles");
    let baseline = free
        .solve_with(&SolveOptions {
            time_limit: Some(Duration::from_secs(5)),
            ..SolveOptions::default()
        })
        .solution
        .expect("the free instance is solvable");

    // Block the slot the most lessons sit in.
    let mut load: BTreeMap<i64, usize> = BTreeMap::new();
    for start in starts(&baseline).values() {
        *load.entry(*start).or_default() += 1;
    }
    let (&blocked, &displaced) = load.iter().max_by_key(|(_, count)| **count).unwrap();
    let restricted = compile(&problem(Some(blocked))).expect("compiles");
    assert!(
        !restricted.check(&baseline).is_feasible,
        "the block must break the baseline"
    );

    let repaired = restricted
        .repair_with_outcome(
            &baseline,
            &RepairOptions {
                time_limit: Duration::from_secs(5),
                ..RepairOptions::default()
            },
        )
        .result;
    let solution = repaired.solution.unwrap_or_else(|| {
        let left = repaired
            .best_effort
            .as_ref()
            .map(|reached| restricted.check(reached).hard_violations.len());
        panic!("no repair of {displaced} displaced lessons; conflicts left: {left:?}")
    });
    assert!(restricted.check(&solution).is_feasible);

    let before = starts(&baseline);
    let moved = starts(&solution)
        .iter()
        .filter(|(activity, start)| before.get(activity) != Some(start))
        .count();
    println!("{displaced} in the blocked slot, {moved} moved");
    // Every displaced lesson moves; chains move some more, but nowhere near a re-solve.
    assert!(moved >= displaced);
    assert!(
        moved <= displaced * 4,
        "{moved} moves for {displaced} displaced lessons"
    );
}
