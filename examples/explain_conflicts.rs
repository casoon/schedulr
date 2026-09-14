//! What schedulr reports when a plan cannot work: structured conflicts from
//! `explain` for infeasible problems, and `CompileError` messages for invalid
//! input. Uses only the API released in schedulr 0.8.0.
//!
//! Run with `cargo run --example explain_conflicts`.

use schedulr::{
    Activity, ActivityId, Conflict, Participant, ParticipantId, Resource, ResourceId,
    ResourceRequirement, SchedulingProblem, TimeWindow, compile,
};

fn main() {
    println!("1. Two fixed lab sessions overlap in the same lab, with the same trainer");
    let lab = Resource::new(ResourceId(1), "Lab 1", 1);
    let chris = Participant::new(ParticipantId(1), "Chris");
    let group_a = Activity::new(ActivityId(1), "Lab group A", TimeWindow::new(9, 11), 2)
        .with_requirement(ResourceRequirement::new(lab.id(), 1))
        .with_participant(chris.id());
    let group_b = Activity::new(ActivityId(2), "Lab group B", TimeWindow::new(10, 12), 2)
        .with_requirement(ResourceRequirement::new(lab.id(), 1))
        .with_participant(chris.id());
    let compiled = compile(&SchedulingProblem::new(
        vec![lab],
        vec![chris],
        vec![group_a, group_b],
    ))
    .expect("the problem compiles");
    let result = compiled.solve();
    println!("   Status: {:?}", result.status);
    print_conflicts(&compiled.explain(&result));
    println!();

    println!("2. Three sessions at 09:00 need a room, but there are only two rooms");
    let rooms = vec![
        Resource::new(ResourceId(1), "Room Aurora", 1).with_type("room"),
        Resource::new(ResourceId(2), "Room Birch", 1).with_type("room"),
    ];
    let sessions = (1..=3)
        .map(|id| {
            Activity::new(
                ActivityId(id),
                format!("Session {id}"),
                TimeWindow::new(9, 10),
                1,
            )
            .with_requirement(ResourceRequirement::matching("room", 1))
        })
        .collect();
    let compiled =
        compile(&SchedulingProblem::new(rooms, vec![], sessions)).expect("the problem compiles");
    let result = compiled.solve();
    println!("   Status: {:?}", result.status);
    print_conflicts(&compiled.explain(&result));
    println!();

    println!("3. Invalid input is rejected before any solving");
    let room = Resource::new(ResourceId(1), "Room Aurora", 0);
    let broken = vec![
        Activity::new(ActivityId(1), "Too long", TimeWindow::new(9, 10), 2)
            .with_requirement(ResourceRequirement::new(ResourceId(1), 1)),
        Activity::new(ActivityId(2), "Missing room", TimeWindow::new(9, 12), 1)
            .with_requirement(ResourceRequirement::new(ResourceId(9), 1)),
        Activity::new(ActivityId(2), "Duplicate id", TimeWindow::new(9, 12), 1),
    ];
    match compile(&SchedulingProblem::new(vec![room], vec![], broken)) {
        Ok(_) => println!("   compiled unexpectedly"),
        Err(error) => {
            for message in error.messages() {
                println!("   CompileError: {message}");
            }
        }
    }
}

fn print_conflicts(conflicts: &[Conflict]) {
    for conflict in conflicts {
        let involved = conflict
            .involved
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "   {:?} {} [{}]: {}",
            conflict.severity, conflict.constraint_name, involved, conflict.message
        );
    }
}
