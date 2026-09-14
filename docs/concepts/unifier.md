---
title: Built on unifier
description: schedulr is the domain layer of a three-crate solver stack. It compiles scheduling concepts into unifier's constraint model and keeps the solver internals out of its public API.
order: 1
---

## The stack

```text
pathwise → unifier → schedulr → (application: timetabling, appointment booking, ...)
```

- [pathwise](https://github.com/casoon/pathwise) provides generic search and optimisation traits
  and strategies.
- [unifier](https://casoon.github.io/unifier/) is a constraint satisfaction and optimisation
  (CSP/COP) framework: integer variables with domains, constraints, weighted objectives and
  several solvers. Its source is at [github.com/casoon/unifier](https://github.com/casoon/unifier).
- schedulr adds the scheduling vocabulary on top: activities, resources, participants, calendars,
  preferences, conflicts.

schedulr depends on `unifier` and nothing else: the 0.8.0 release on unifier 0.3.1, the master
branch on unifier 0.3.2.

## What schedulr compiles into

`compile` walks the `SchedulingProblem` and builds a unifier model with its `ModelBuilder`:

| schedulr | unifier |
| --- | --- |
| `Activity` with window and duration | an interval: start and end variables, fixed duration |
| `Resource` with capacity 1 | `NoOverlap` over the activities using it |
| `Resource` with capacity above 1 | `Cumulative` over the activities using it |
| `Participant` | a resource with capacity 1 |
| flexible requirement (type, pool, candidates) | one presence variable per candidate, `ExactlyOne` of them, plus schedulr's own `AlternativeResourceCapacity` constraint per candidate |
| `ScheduleTemplate` | a `PeriodicValues` restriction on each start |
| unavailable ranges | `ForbiddenValues` on the start, optional per candidate |
| `SameStart`, `Consecutive` | `Equal` on start/start or end/start |
| `Precedence { min_gap }` | `Precedence` |
| `NoOverlap` relation | `NoOverlap` over the two intervals |
| `ScoreRule` | a scored objective on its level |

Participant groups are expanded to their members before this step, so unifier only ever sees
individual participants.

## Solvers

schedulr picks the solver per call:

- `solve()` without score rules: unifier's `BacktrackingSolver`.
- `solve()` with score rules: unifier's `BranchAndBoundSolver`.
- `repair()`: unifier's `LnsSolver`, started from the baseline.
- `SchedulingState` and `evaluate_move`: no solver, only unifier's incremental check of the
  constraints adjacent to the changed variables.

## What stays hidden

No unifier type appears in schedulr's public API. Variables, constraint ids and unifier's
violation structs are mapped back to `ActivityId`, `ResourceId`, `ParticipantId` and the names
you gave, so a `Conflict` can be shown to users directly. If you need constraints schedulr does
not model, work with unifier directly; the [unifier documentation](https://casoon.github.io/unifier/)
describes its model and solvers.
