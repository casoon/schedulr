# Changelog

All notable changes to schedulr are listed here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and schedulr follows
[Semantic Versioning](https://semver.org/): before 1.0, a minor version can contain breaking
changes.

This file starts with 0.14.0; earlier releases are recorded by their git tags.

## [Unreleased]

### Fixed

- **Explain:** `analyze` spreads demand over candidate resources and participants as exact
  shares instead of rounding each share up, so a one-slot activity with many candidate rooms no
  longer counts as a full slot in every one of them.

## [0.14.0] - 2026-10-24

### Breaking

- **Dependencies:** schedulr builds on `unifier` 0.6 instead of 0.5. The re-exported
  `CancellationToken` is now the `pathwise` 0.3 type that unifier 0.6 uses.

### Added

- **Day rules:** `DayRule` with `DayRuleKind` shapes one entity's occupancy per period bucket —
  minimum load, maximum span, a break between buckets, a free run inside a window, and a limit
  on sections of the window (`SchedulingProblem::with_day_rule`).
- **Day rules:** `MaximumDistinctPerBucket` limits how many distinct groups of activities start in
  one bucket, e.g. at most five subjects a day (`with_distinct_per_bucket`).
- **Goals:** `IsolatedBlockGoal`, `JointPresenceGoal` and `ConcurrentFreeGoal` score lone blocks,
  days without shared activities and too many entities free at once.
- **Goals:** `ActivityGoal` (`ActivityGoalKind`, `ProximityMode`) scores the distribution of a set
  of activities across buckets.
- **Goals:** `ResourceCostGoal` ranks the resources activities use (fallback rooms, a home room);
  `RoomChangeGoal` scores resource changes and travel between locations.
- **Goals:** `IdleBounds` keeps one entity's free time inside a band and penalizes long gaps;
  `TimePreference` scores occupancy of wanted or unwanted time windows.
- **Soft rules:** `SoftPenalty` turns a hard rule — day rules, daily load, bucket patterns, bucket
  starts, active buckets, minimum breaks, activity relations — into a preference via `soft`.
- **Model:** `Activity::with_allowed_window` restricts where an activity may start.
- **Model:** `ResourceRequirement::with_minimum_capacity_in` checks a minimum size such as seats
  against a named capacity dimension; `units` stays the number of parallel uses. Details:
  [Modeling](https://casoon.github.io/schedulr/docs/guides/modeling/).

### Changed

- **Solving:** resource capacity counts every unit over the limit instead of one violation per
  resource, so repairs no longer pile up clashes on one room or teacher; `explain` names only
  the overlapping activities.
- **Solving:** free-time, distribution and resource-capacity scoring answer a move locally
  (`Objective::delta`, `Constraint::move_violations`), several times more steps per second in
  timetable repairs.
- **Explain:** `analyze` counts participants from requirements, with demand spread over the
  candidates.
- **Contributors:** ITC-2007 Track 3 instances (comp01, comp11) as an external benchmark with
  independent scoring; a repair test for a period blocked in a full timetable.
