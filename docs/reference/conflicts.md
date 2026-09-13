---
title: Conflicts and errors
description: The structure of a Conflict, which constraint names you will see and what they mean, and the validation messages of CompileError.
order: 2
---

## Conflict

`explain`, `SchedulingState::check_feasibility`, `commit` and `evaluate_move` report problems as
`Conflict` values:

| Field | Content |
| --- | --- |
| `severity` | `Blocking` or `Advisory` |
| `constraint_name` | Name of the violated constraint, see below |
| `involved` | The `ActivityId`s taking part |
| `entity` | The `Resource`, `Participant` or `Activity` concerned, if any |
| `message` | Readable text, prefixed with the resource or participant name |

Conflicts on resources are `Blocking`. Conflicts on participants are `Advisory`: a booking desk
may accept them. Batch `solve` still treats participant double-booking as a hard constraint and
never returns a plan with it.

## Constraint names

| Name | Raised when |
| --- | --- |
| `NoOverlap` | Two activities overlap on a resource with capacity 1, on a participant, or across a `NoOverlap` relation |
| `Cumulative` | Activities on a resource with capacity above 1 exceed that capacity |
| `AlternativeResourceCapacity` | The candidates of flexible requirements cannot hold all activities |
| `Precedence` | A `Precedence` relation is violated |
| `Equal` | A `SameStart` or `Consecutive` relation is violated |
| `PeriodicValues` | A start lies outside the slot calendar or on an exception |
| `ForbiddenValues` | A start lies in an unavailable range of a resource or participant |
| `Model` | `SchedulingState` received invalid input, for example an unknown activity to exclude |
| `ActivityDomain` | `evaluate_move` got an unknown activity or a window outside the activity's domain |

Messages for overlaps name the intervals, for example:

```text
Blocking NoOverlap [a1, a2]: Lab 1: Intervals [9, 11) and [10, 12) overlap
Advisory NoOverlap [a1, a2]: Chris: Intervals [9, 11) and [10, 12) overlap
```

## CompileError

`compile` validates the whole problem first and returns every problem at once.
`CompileError::messages()` lists them; `Display` joins them with `; `. The checks:

- duplicate resource, participant or activity ids;
- a resource with zero capacity;
- an activity with an invalid window (end not after start, window shorter than the duration, or
  duration 0);
- unknown resources, participants, groups, pools or subgroups in activities, memberships and pools;
- a zero-unit resource requirement;
- a requirement with no matching resource or participant;
- a participant group cycle;
- a schedule template with a cycle length of 0 or less, or without any slot;
- an activity outside the academic period;
- a score rule for an unknown activity or with a weight of 0 or less;
- an activity relation with an unknown activity, or relating an activity to itself.

From the [conflicts example](../../../showcase/explain-conflicts/):

```text
CompileError: resource r1 has zero capacity
CompileError: activity a1 has an invalid time window
CompileError: activity a2 requires unknown resource r9
CompileError: duplicate activity id a2
```
