# schedulr

Scheduling framework (activity/resource/interval DSL) for Rust, built
on [`unifier`](https://github.com/casoon/unifier)'s CSP/COP constraint
model and solvers, which in turn build on
[`pathwise`](https://github.com/casoon/pathwise)'s generic search and
optimization traits.

```
pathwise → unifier → schedulr → (application: timetabling, appointment booking, ...)
```

## Status

Version 0.1 is implemented. It provides:

- domain-neutral `Resource`, `Participant`, `Activity`, `Assignment`,
  `Score`, and structured `Conflict` types without exposing solver internals;
- synchronous create/move/cancel/participant-update checks through
  `SchedulingState`, including self-exclusion when moving an activity;
- a minimal `compile` → `solve` → `explain` batch path for hard resource
  conflicts.

The single-activity path evaluates only constraints affected by the proposed
change and does not start a solver search. Persistence, recurring calendars,
repair modes, and richer score categories remain application or later-version
concerns; see `plan/02-umsetzungsplan.md`.

## Problem class

Scheduling / timetabling / appointment booking on top of `unifier`'s
CSP/COP model: activities placed against resources over time, subject
to hard constraints (no double-booking, capacity, precedence,
calendar/opening-hours exclusions) and soft preferences (e.g.
proximity to a requested time slot).

## Usage

Synchronous single-activity checks (e.g. a live booking desk), via
`SchedulingState` — no solver search, only the constraints touching the
proposed activity are evaluated:

```rust
use schedulr::{
    Participant, ParticipantId, ProposedActivity, Resource, ResourceId,
    ResourceRequirement, SchedulingState, TimeWindow,
};

let mut state = SchedulingState::new(
    [Resource::new(ResourceId(1), "room", 1)],
    [Participant::new(ParticipantId(1), "Alex")],
);

let proposal = ProposedActivity::new("appointment", TimeWindow::new(10, 20))
    .with_requirement(ResourceRequirement::new(ResourceId(1), 1))
    .with_participant(ParticipantId(1));

// Blocking conflicts (e.g. room double-booked) prevent commit; advisory
// conflicts (e.g. a participant already booked elsewhere) do not.
let activity_id = state.commit(proposal).expect("room and participant are free");

// Moving an activity excludes its own prior booking from the check via
// `excluding`, so it does not conflict with itself:
let moved = ProposedActivity::new("appointment", TimeWindow::new(15, 25))
    .with_requirement(ResourceRequirement::new(ResourceId(1), 1))
    .with_participant(ParticipantId(1))
    .excluding(activity_id);
state.commit(moved).expect("new slot is free");
```

Batch scheduling with a minimal `compile` → `solve` → `explain` path for
hard resource conflicts:

```rust
use schedulr::{
    Activity, ActivityId, Resource, ResourceId, ResourceRequirement,
    SchedulingProblem, SolveStatus, TimeWindow, compile,
};

let room = Resource::new(ResourceId(1), "Physics lab", 1);
let first = Activity::new(ActivityId(1), "first", TimeWindow::new(10, 20), 10)
    .with_requirement(ResourceRequirement::new(room.id(), 1));
let second = Activity::new(ActivityId(2), "second", TimeWindow::new(15, 25), 10)
    .with_requirement(ResourceRequirement::new(room.id(), 1));

let compiled = compile(&SchedulingProblem::new(vec![room], vec![], vec![first, second]))
    .expect("problem compiles");
let result = compiled.solve();
if result.status != SolveStatus::Feasible {
    for conflict in compiled.explain(&result) {
        println!("{}: {}", conflict.constraint_name, conflict.message);
    }
}
```

## Installation

Not published to crates.io yet.

## License

MIT — see [LICENSE](LICENSE).
