---
title: Quickstart
description: Describe two activities that compete for one room, solve the problem, and read either the plan or the explanation.
order: 2
---

## Describe the problem

Every problem starts with resources, participants and activities. Here one lab can host one
activity at a time, and two activities of ten time units each want it:

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

let problem = SchedulingProblem::new(vec![room], vec![], vec![first, second]);
```

`TimeWindow::new(10, 20)` with a duration of 10 leaves exactly one possible start, 10. The second
activity can only start at 15. Both need the lab at the same time.

## Compile and solve

```rust
let compiled = compile(&problem).expect("problem compiles");
let result = compiled.solve();
```

`compile` validates the input and builds the constraint model; errors come back as one
`CompileError` with all messages. `solve` runs the search and returns a `SolveResult`.

## Read the result

```rust
match result.status {
    SolveStatus::Feasible => {
        for assignment in &result.solution.as_ref().unwrap().assignments {
            println!("{} at {:?} on {:?}", assignment.activity, assignment.window, assignment.resources);
        }
    }
    _ => {
        for conflict in compiled.explain(&result) {
            println!("{:?} {}: {}", conflict.severity, conflict.constraint_name, conflict.message);
        }
    }
}
```

This problem is infeasible. `explain` names the constraint, the activities involved and the
resource by name. The same situation in the [conflicts example](../../../showcase/explain-conflicts/)
prints:

```text
Blocking NoOverlap [a1, a2]: Lab 1: Intervals [9, 11) and [10, 12) overlap
```

Widen one of the windows, for example `TimeWindow::new(15, 35)`, and `solve` places the second
activity after the first.

## Next steps

- [The scheduling DSL](../../guides/modeling/): flexible resources, pools and groups.
- [Calendars and availability](../../guides/calendars/): slot templates and exceptions.
- [Booking single activities](../../guides/booking-desk/): checks without a solver search.
