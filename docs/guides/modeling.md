---
title: The scheduling DSL
sidebarLabel: The scheduling DSL
description: A schedulr problem is plain Rust data built with small builder methods. Resources and participants are what gets booked; activities are the demand; requirements connect the two.
order: 1
---

schedulr does not have its own file format or parser. The "DSL" is a set of domain types with
`with_*` builder methods: you describe what has to happen, and schedulr compiles it into a
constraint model for [unifier](https://casoon.github.io/unifier/). Nothing in the public types
refers to solver variables or constraint ids.

## Time

Time is an integer. `TimeWindow::new(start, end)` is the half-open interval `[start, end)`;
`duration()` returns `end - start`. What one unit means is up to you: the examples on this site
use one hour or one minute. Pick the coarsest unit your application can live with, because the
solver searches over individual values.

## Resources

A `Resource` is anything with capacity that activities consume: a room, a machine, a vehicle.

```rust
use schedulr::{Resource, ResourceId};

let lab = Resource::new(ResourceId(3), "Lab 1", 12)
    .with_type("lab")
    .with_feature("workstations");
```

- `capacity` is the number of units that can be in use at the same time. A capacity of 1 means
  the resource can host one activity at a time.
- `with_type` and `with_feature` are what matching requirements look for (see below). The type
  defaults to `"resource"`.
- `with_attribute` stores free-form key/value data for your application.
- `with_capacity_dimension` stores additional named capacities. The solver currently enforces
  only the default `units` dimension.

A `ResourcePool` names a set of interchangeable resources, for example "Seminar rooms".

## Participants and groups

A `Participant` is a person (or anything else) that should not be in two places at once.

```rust
use schedulr::{Participant, ParticipantId};

let ana = Participant::new(ParticipantId(1), "Ana");
```

Participants can be organised in two ways:

- `ParticipantPool`: a named set of interchangeable participants, for example all trainers who
  can teach a topic. An activity can ask for one participant from a pool.
- `ParticipantGroup` plus `GroupMembership`: a group of participants who attend together, such
  as a class or a team. Groups can contain subgroups and can overlap. When an activity uses a
  group, schedulr expands it to every member, recursively; a membership cycle is a compile
  error.

## Activities

An `Activity` is one thing to schedule. It has an id, a name, the window it may be placed in,
and a duration:

```rust
use schedulr::{Activity, ActivityId, ParticipantGroupId, ResourceRequirement, TimeWindow};

let kickoff = Activity::new(ActivityId(1), "Kickoff", TimeWindow::new(0, 48), 1)
    .with_requirement(ResourceRequirement::matching("room", 1).with_minimum_capacity(20))
    .with_participant_group(ParticipantGroupId(1));
```

The solver picks a start inside the allowed window so that the whole activity fits: an activity
with window `[0, 48)` and duration 1 can start anywhere from 0 to 47.

Participants are attached in three ways:

| Builder | Meaning |
| --- | --- |
| `with_participant(id)` | This participant attends. |
| `with_participant_group(id)` | Every member of the group attends. |
| `with_participant_requirement(req)` | One participant, chosen by the solver. |

## Resource requirements

A `ResourceRequirement` says how many units of which resource an activity needs. It is either
exact or resolved by the solver:

| Constructor | The solver uses |
| --- | --- |
| `ResourceRequirement::new(id, units)` | exactly this resource |
| `ResourceRequirement::matching(type, units)` | one resource of this type |
| `ResourceRequirement::from_pool(pool, units)` | one resource from this pool |

Flexible requirements can be narrowed further with `with_feature` (the resource must have all
listed features), `with_minimum_capacity` and `with_candidate` (an explicit allow list). If no
resource matches, `compile` fails with "has no matching resource for a requirement".

`ParticipantRequirement` works the same way for people: `new(id)`, `matching()` (any
participant, usually narrowed with `with_candidate`) and `from_pool(pool)`.

## The problem

`SchedulingProblem` bundles everything:

```rust
use schedulr::{SchedulingProblem, compile};

let problem = SchedulingProblem::new(resources, participants, activities)
    .with_resource_pool(seminar_rooms)
    .with_participant_pool(rust_trainers)
    .with_participant_group(trainers)
    .with_group_membership(membership);

let compiled = compile(&problem)?;
let result = compiled.solve();
```

Calendars, score rules and activity relations are added the same way; they have their own
pages: [Calendars and availability](../calendars/) and
[Relations and scores](../relations-and-scores/).

`compile` validates the whole problem before building the model: duplicate ids, zero capacities,
invalid windows, unknown references and empty candidate sets are collected into one
`CompileError`. See [Conflicts and errors](../../reference/conflicts/) for the full list.

## The solution

`solve()` returns a `SolveResult` with a `status` (`Feasible`, `Infeasible` or `Aborted`), an
optional `Solution` and `statistics`. A `Solution` holds one `Assignment` per activity (window,
chosen resources, participants), the aggregated `Score` and one `ScoreComponent` per score rule.

The [workshop plan](../../../showcase/workshop-plan/) in the showcase puts all of the above
together.
