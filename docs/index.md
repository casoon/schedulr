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

## Versions

The latest release on crates.io is **0.11.0**, and master carries nothing beyond it.

0.11.0 adds `SolveResult::best_effort`: how far a run got when it could not return a schedule —
a complete assignment that breaks hard constraints, deliberately kept out of `solution` so no
caller can mistake it for one. Hand it to `CompiledProblem::check` for the reasons, in the same
`Conflict` vocabulary as every other explanation here. Breaking only because the struct gained a
public field.

Two paths used to drop that information on the floor: a run whose assignment the hard-constraint
gate rejected kept nothing, and the two-phase search fell back to its construction phase and
discarded whatever the optimizer had reached. Both now carry it through.

0.10.0 moves to unifier 0.5 and is breaking only through it: `CancellationToken` is a unifier
type re-exported here, and the hard score is now a count of *violations* rather than of violated
constraints, so its magnitudes differ from 0.9.0's for the same schedule. Feasibility is
unchanged — a hard score of zero still means every hard constraint holds. What the release is
worth lies in unifier: a search now reports the best complete assignment it reached even when it
could not vouch for it, and the construction that feeds it undoes conflicts instead of building
on them.

0.9.0 added what 0.8.0 lacked: breaks, per participant and per resource availability, activity
relations, baseline-aware repair, participant choice groups and `CompiledProblem::check`. A run
also constructs a feasible schedule before optimizing it, chooses how to spend its budget (see
`SolveStrategy`), and never returns a schedule that violates hard constraints.

## Where it stops

- Time is an integer. Mapping it to dates, time zones and clock times is the application's job.
- There is no file format, serialization or CLI; problems are built in Rust.
- Persistence is an application concern. Solutions carry their score components so they can be
  stored with each schedule version.
- The solver is an exact search with a time limit. It is not yet tested on production-scale
  instances, so evaluate it on your own data before relying on it.

## How the docs are organised

- **Getting started**: install the crate and solve a first problem.
- **Guides**: the scheduling DSL, calendars and availability, relations and scores, booking single
  activities, analysis and repair.
- **Concepts**: how schedulr builds on unifier.
- **Reference**: API overview and the conflicts and errors schedulr reports. Item-level
  documentation lives on [docs.rs](https://docs.rs/schedulr).
