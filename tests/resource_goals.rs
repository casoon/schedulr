//! Resource costs and room changes (timbra plan/69, package C).

use schedulr::{
    Activity, ActivityId, DayTemplate, EntityRef, Participant, ParticipantId, Resource,
    ResourceCostGoal, ResourceId, ResourceRequirement, RoomChangeGoal, ScheduleTemplate,
    SchedulingProblem, ScoreLevel, SlotTemplate, Solution, TimeWindow, compile,
};

const P: ParticipantId = ParticipantId(1);
const A: ResourceId = ResourceId(1);
const B: ResourceId = ResourceId(2);

fn rooms() -> Vec<Resource> {
    vec![
        Resource::new(A, "A", 1).with_type("room"),
        Resource::new(B, "B", 1).with_type("room"),
    ]
}

fn either_room() -> ResourceRequirement {
    ResourceRequirement::matching("room", 1)
        .with_candidate(A)
        .with_candidate(B)
}

fn one_day() -> ScheduleTemplate {
    let day = (0..4).fold(DayTemplate::new(0), |day, slot| {
        day.with_slot(SlotTemplate::new(format!("s{slot}"), slot, 1))
    });
    ScheduleTemplate::new(4).with_day(day)
}

fn solve(problem: SchedulingProblem) -> Solution {
    compile(&problem)
        .expect("compiles")
        .solve()
        .solution
        .expect("solves")
}

fn room_of(solution: &Solution, id: u64) -> Vec<ResourceId> {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == ActivityId(id))
        .expect("placed")
        .resources
        .clone()
}

/// A ranking: room B costs nothing, room A one — the lesson takes B.
#[test]
fn the_cheaper_room_wins() {
    let lesson =
        Activity::new(ActivityId(1), "l", TimeWindow::new(0, 4), 1).with_requirement(either_room());
    let solution = solve(
        SchedulingProblem::new(rooms(), vec![], vec![lesson]).with_resource_cost_goal(
            ResourceCostGoal {
                category: "Rangfolge".to_string(),
                level: ScoreLevel::Medium,
                weight: 1,
                activities: vec![ActivityId(1)],
                costs: vec![(A, 1), (B, 0)],
                default_cost: 0,
            },
        ),
    );
    assert_eq!(room_of(&solution, 1), vec![B]);
    assert_eq!(solution.score.medium, 0);
}

/// A person's first lesson is fixed in room A; the second one, free to take A or B, stays in A to
/// avoid a room change.
#[test]
fn a_room_change_is_avoided() {
    let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 1), 1)
        .with_participant(P)
        .with_requirement(ResourceRequirement::new(A, 1));
    let second = Activity::new(ActivityId(2), "second", TimeWindow::new(1, 2), 1)
        .with_participant(P)
        .with_requirement(either_room());
    let solution = solve(
        SchedulingProblem::new(rooms(), vec![Participant::new(P, "P")], vec![first, second])
            .with_schedule_template(one_day())
            .with_room_change_goal(RoomChangeGoal {
                category: "Raumwechsel".to_string(),
                level: ScoreLevel::Medium,
                entity: EntityRef::Participant(P),
                locations: vec![(A, 0), (B, 1)],
                travel_gap: 1,
                change_weight: 1,
                travel_weight: 5,
            }),
    );
    assert_eq!(room_of(&solution, 2), vec![A]);
    assert_eq!(solution.score.medium, 0);
}

/// A planned lesson changes rooms without moving: the replacement is checked like any change, and a
/// room outside the requirement's candidates is refused.
#[test]
fn a_room_can_be_replaced_in_a_plan() {
    use schedulr::ChangeRequest;
    let lesson =
        Activity::new(ActivityId(1), "l", TimeWindow::new(0, 4), 1).with_requirement(either_room());
    let compiled =
        compile(&SchedulingProblem::new(rooms(), vec![], vec![lesson])).expect("compiles");
    let solution = compiled.solve().solution.expect("solves");
    let now = room_of(&solution, 1)[0];
    let other = if now == A { B } else { A };

    let moved = compiled.evaluate_changes(
        &solution,
        &[ChangeRequest::ReplaceResource {
            activity: ActivityId(1),
            from: now,
            to: other,
        }],
    );
    assert!(moved.is_feasible);
    assert_eq!(room_of(&moved.candidate, 1), vec![other]);
    assert_eq!(
        moved.candidate.assignments[0].window, solution.assignments[0].window,
        "the time stays"
    );

    let refused = compiled.evaluate_changes(
        &solution,
        &[ChangeRequest::ReplaceResource {
            activity: ActivityId(1),
            from: now,
            to: ResourceId(99),
        }],
    );
    assert!(!refused.is_feasible);
}
