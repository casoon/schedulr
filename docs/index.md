---
title: Overview
description: What schedulr does, how it relates to unifier, where it stops, and how this documentation is organised.
order: 0
---

schedulr is a scheduling framework for Rust: timetabling, appointment booking, workshop and
shift planning. You describe the problem with domain types (activities, resources, participants,
time windows, calendars, preferences); schedulr turns that description into a constraint model
and solves it with [unifier](https://casoon.github.io/unifier/).

```text
pathwise → unifier → schedulr → (application: timetabling, appointment booking, ...)
```

## What it covers

- Batch planning: `compile` a `SchedulingProblem`, `solve` it, and read one assignment per
  activity with its window, resources and participants.
- Hard constraints: no double-booking, resource capacity, resource matching by type, feature and
  capacity, pools, participant groups, periodic slot calendars with exceptions.
- Preferences on three lexicographic levels, with a score component per rule in every solution.
- Single-activity checks for booking UIs through `SchedulingState`, without a solver search.
- Explanations and follow-up work: `explain`, `analyze`, `evaluate_move`, `suggest`, `compare` and
  a baseline-aware `repair`.

## How a run works

`solve()` first constructs a schedule that satisfies every hard constraint, then spends the rest
of its budget improving it; `SolveOptions` sets the time limit, a cancellation token and the
`SolveStrategy`. A schedule that violates hard constraints is never returned as a solution: when
a run gets no further than that, the assignment it reached is kept in `SolveResult::best_effort`,
and `CompiledProblem::check` gives the reasons in the same `Conflict` vocabulary as every other
explanation here. See [Analysis and repair](guides/analysis-and-repair/#solving).

## Where it stops

- Time is an integer. Mapping it to dates, time zones and clock times is the application's job.
- There is no file format, serialization or CLI; problems are built in Rust.
- Persistence is an application concern. Solutions carry their score components so they can be
  stored with each schedule version.
- The search runs within a time limit and proves optimality only when it can. It is not yet
  tested on production-scale instances, so evaluate it on your own data before relying on it.

## How the docs are organised

- **Getting started**: install the crate and solve a first problem.
- **Guides**: the scheduling DSL, calendars and availability, relations and scores, booking single
  activities, analysis and repair.
- **Concepts**: how schedulr builds on unifier.
- **Reference**: API overview and the conflicts and errors schedulr reports. Item-level
  documentation lives on [docs.rs](https://docs.rs/schedulr).
