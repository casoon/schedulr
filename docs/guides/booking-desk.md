---
title: Booking single activities
sidebarLabel: Booking single activities
description: SchedulingState answers "does this one appointment fit?" without starting a solver search. Create, move, cancel and change participants of committed activities.
order: 4
---

Batch solving plans a whole problem at once. A booking desk, a calendar UI or an API endpoint
usually needs something else: check one proposed change against what is already booked, and
answer immediately. That is `SchedulingState`.

## Set up the state

```rust
use schedulr::{Participant, ParticipantId, Resource, ResourceId, SchedulingState};

let mut state = SchedulingState::new(
    [Resource::new(ResourceId(1), "Consulting room", 1)],
    [
        Participant::new(ParticipantId(1), "Alex"),
        Participant::new(ParticipantId(2), "Blair"),
    ],
    [], // resource pools
);
```

The state holds resources, participants, resource pools and the committed activities, indexed
by resource and participant. Persistence is left to your application.

## Propose, check, commit

A `ProposedActivity` has a name, an exact `TimeWindow`, participants and resource requirements:

```rust
use schedulr::{ProposedActivity, ResourceRequirement, TimeWindow};

let proposal = ProposedActivity::new("consultation", TimeWindow::new(600, 660))
    .with_requirement(ResourceRequirement::new(ResourceId(1), 1))
    .with_participant(ParticipantId(1));

let conflicts = state.check_feasibility(&proposal); // Vec<Conflict>, empty if it fits
let id = state.commit(proposal)?;                   // Result<ActivityId, Vec<Conflict>>
```

`check_feasibility` builds a model of only the committed activities that share a resource or a
participant with the proposal and evaluates the constraints touching the proposal. No solver
search runs.

`commit` runs the same check and refuses the proposal if any conflict is `Blocking`. `Advisory`
conflicts are returned by the check but do not block the commit.

## Blocking and advisory

| Conflict on | Severity |
| --- | --- |
| a resource (room double-booked, capacity exceeded) | `Blocking` |
| a participant (person already busy) | `Advisory` — or `Blocking` if the problem asks for it |
| invalid input (unknown activity, invalid window) | `Blocking`, constraint name `Model` |

The distinction is deliberate: a room cannot hold two appointments, but a person can decide to be
double-booked. Your application decides whether to show a warning or refuse. An application whose
people *cannot* be in two places at once passes
`SchedulingProblem::with_participant_conflict_policy(ParticipantConflictPolicy::Blocking)` — see
[Conflicts and errors](../reference/conflicts.md).

## Move, change and cancel

- **Move:** build a proposal for the new window and call `excluding(id)`. The activity's own
  earlier booking is then ignored, so it does not conflict with itself. Committing replaces the
  activity under the same id.
- **Change participants:** `proposal_for(id)` returns the committed activity as a proposal that
  already excludes itself; add participants with `add_participant` and commit it.
- **Cancel:** `cancel(id)` removes the activity and returns it with its assignment.

`activity(id)`, `assignment(id)`, `len()` and `is_empty()` read the current state.

## A full session

The [booking desk](../../../showcase/booking-desk/) showcase runs these steps against one room
and two people. Its captured output:

```text
2. Check a second consultation in the same room, 10:30-11:30
   Blocking NoOverlap [a0, a1]: Consulting room: Intervals [600, 660) and [630, 690) overlap
   Advisory NoOverlap [a0, a1]: Alex: Intervals [600, 660) and [630, 690) overlap
```

The room conflict blocks, the participant conflict is a warning. Adding Blair, who already has a
call at the same time, produces only an advisory conflict, and the commit goes through.
