//! A booking desk: create, check, move and cancel single appointments with
//! `SchedulingState`. No solver search runs; each check only evaluates the
//! constraints touching the proposed appointment. Uses only the API released
//! in schedulr 0.8.0.
//!
//! One time unit is one minute; minute 0 is 00:00.
//!
//! Run with `cargo run --example booking_desk`.

use schedulr::{
    Conflict, Participant, ParticipantId, ProposedActivity, Resource, ResourceId,
    ResourceRequirement, SchedulingState, TimeWindow,
};

fn main() {
    let room = ResourceId(1);
    let alex = ParticipantId(1);
    let blair = ParticipantId(2);
    let mut state = SchedulingState::new(
        [Resource::new(room, "Consulting room", 1)],
        [
            Participant::new(alex, "Alex"),
            Participant::new(blair, "Blair"),
        ],
        [],
    );
    let consultation = |start: i64, end: i64| {
        ProposedActivity::new("consultation", TimeWindow::new(start, end))
            .with_requirement(ResourceRequirement::new(room, 1))
            .with_participant(alex)
    };

    println!("1. Book a consultation with Alex, 10:00-11:00");
    let first = state
        .commit(consultation(600, 660))
        .expect("room and Alex are free");
    println!("   committed as {first}");

    println!("2. Check a second consultation in the same room, 10:30-11:30");
    print_conflicts(&state.check_feasibility(&consultation(630, 690)));

    println!("3. Book a call for Blair without a room, 10:00-11:00");
    let call = state
        .commit(ProposedActivity::new("call", TimeWindow::new(600, 660)).with_participant(blair))
        .expect("Blair is free");
    println!("   committed as {call}");

    println!("4. Add Blair to the consultation");
    let mut with_blair = state.proposal_for(first).expect("consultation exists");
    with_blair.add_participant(blair);
    print_conflicts(&state.check_feasibility(&with_blair));
    match state.commit(with_blair) {
        Ok(id) => println!("   committed anyway: advisory conflicts do not block ({id})"),
        Err(conflicts) => print_conflicts(&conflicts),
    }

    println!("5. Move the consultation to 10:30-11:30");
    let moved = consultation(630, 690).excluding(first);
    print_conflicts(&state.check_feasibility(&moved));
    let id = state.commit(moved).expect("the new slot is free");
    let window = state.assignment(id).expect("still booked").window;
    println!(
        "   {id} now runs {}-{}",
        clock(window.start),
        clock(window.end)
    );

    println!("6. Cancel Blair's call");
    state.cancel(call).expect("call exists");
    println!("   {} activity left in the state", state.len());
}

fn print_conflicts(conflicts: &[Conflict]) {
    if conflicts.is_empty() {
        println!("   no conflicts");
    }
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

fn clock(minute: i64) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}
