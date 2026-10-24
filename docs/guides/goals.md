---
title: Goals and preferences
sidebarLabel: Goals and preferences
description: Problem-level preferences beyond single-activity score rules — time preferences per person, free time, distribution of a course over the week, room costs and room changes.
order: 5
---

A `ScoreRule` (see [Relations and scores](../relations-and-scores/)) belongs to one activity.
The goals on this page belong to the problem: a person's preferred hours, a teacher's free
periods, how a course spreads over the week. They are never hard. Every goal has

- a `category`, the name it is reported under in `Solution::score_components` (with
  `activity: None`, since no single activity owns it),
- a `level` (`Strong`, `Medium` or `Weak`), compared lexicographically like score rules,
- a `weight`, the cost per counted unit.

A goal's cost lowers the score on its level. Like the rules in
[Day and bucket rules](../day-rules/), goals that count "per bucket" use the buckets of the
schedule template. Offsets count from the start of each bucket window, and a bucket is a concrete
window: Monday of week A and Monday of week B are two buckets.

## Time preferences

`TimePreference::new(category, level, entity, windows)` values the time one entity spends in
given windows. `windows` is a list of `(TimeWindow, value)`: every occupied unit inside a window
scores its `value`, positive for wanted and negative for unwanted time.

```rust
use schedulr::{EntityRef, ParticipantId, ScoreLevel, TimePreference, TimeWindow};

// Ana would rather not teach on Monday morning (one unit = one hour, Monday = [0, 24)).
let problem = problem.with_time_preference(TimePreference::new(
    "Ana: no Monday mornings",
    ScoreLevel::Medium,
    EntityRef::Participant(ParticipantId(1)),
    vec![(TimeWindow::new(8, 12), -1)],
));
```

The entity may be a resource (counted with its requirement's units), a participant (counted once)
or a single activity. Windows are absolute, so a preference for every Monday lists every Monday
of the horizon. Windows of one preference should not overlap; where they do, both values apply.
An unknown resource or participant, or a window ending before it starts, is a compile error.

## Free time

`IdleBounds::new(category, level, entity, min, max, weight)` sets a target band for one entity's
free time. Free time is measured inside each bucket between the entity's first and last
assignment there, and summed over all buckets. Every unit below `min` or above `max` costs
`weight`. `with_long_gaps(length, weight)` additionally charges every single gap of at least
`length` units — "two to four free periods a week, and no double free periods":

```rust
use schedulr::{EntityRef, IdleBounds, ParticipantId, ScoreLevel};

let problem = problem.with_idle_bounds(
    IdleBounds::new("Ben's free periods", ScoreLevel::Weak, EntityRef::Participant(ParticipantId(2)), 2, 4, 1)
        .with_long_gaps(2, 5),
);
```

Without `with_long_gaps`, gap lengths cost nothing.

## Soft goals

`SoftGoal::new(category, level, weight, kind)` adds one of four global preferences:

| `SoftGoalKind` | Minimises |
| --- | --- |
| `MinimizeParticipantIdle` | Free time between each participant's assignments inside a bucket |
| `MinimizeGroupIdle` | Free time in the combined occupancy of each participant group's members inside a bucket |
| `RoomStability` | The number of distinct resources used by activities that share a name |
| `SpreadActivityOverDays` | Activities that share a name clustering in the same bucket |

Unlike `IdleBounds`, `MinimizeParticipantIdle` names no one and has no band: every unit of free
time costs. Use `IdleBounds` when some free time is wanted.

## Distribution of activities

`ActivityGoal::new(category, level, weight, kind)` measures how a set of activities, usually the
occurrences of one course, lies over the buckets:

| `ActivityGoalKind` | Every counted unit is |
| --- | --- |
| `NotOnAdjacentBuckets { activities }` | a pair of consecutive buckets that both see a start — "not on consecutive days" |
| `OffsetVariety { activities }` | a start on an offset another one already uses — "not always in the same period" |
| `OffsetStability { activities }` | a distinct offset beyond the first — "always in the same period" |
| `StartsWithinOffsets { activities, start_offset, end_offset, min, max }` | a start below `min` or above `max` inside `[start_offset, end_offset)` — "at least once before the cut-off period" |
| `Proximity { groups, mode }` | a pair of activities from *different* groups in one bucket, judged by `mode` |
| `BlockAtEdges { activities, min_length, edges }` | a block of at least `min_length` that neither starts nor ends at the bucket's start, its end or one of `edges` |

`ProximityMode` decides what a pair from different groups costs:

| `ProximityMode` | Costs |
| --- | --- |
| `AvoidAdjacent` | directly back to back — "no two of these subjects in a row" |
| `AvoidSameBucket` | sharing a bucket at all — "not on the same day" |
| `PreferAdjacent` | sharing a bucket without touching — "if on the same day, then back to back" |

```rust
use schedulr::{ActivityGoal, ActivityGoalKind, ActivityId, ScoreLevel};

let problem = problem.with_activity_goal(ActivityGoal::new(
    "physics not on consecutive days",
    ScoreLevel::Medium,
    1,
    ActivityGoalKind::NotOnAdjacentBuckets { activities: vec![ActivityId(7), ActivityId(8)] },
));
```

## Resources

`ResourceCostGoal` ranks the resources a set of activities may use: every resource an activity
uses, fixed or chosen by the solver, costs its entry in `costs` (or `default_cost` when it is not
listed), times `weight`. That expresses a home room, fallback rooms, or a room that fits the group.

`RoomChangeGoal` follows one entity through each bucket. Two consecutive activities in different
resources cost `change_weight`. If the two resources have different entries in `locations` and
fewer than `travel_gap` units lie between the activities, `travel_weight` is added. Resources
without a location never cause travel.

```rust
use schedulr::{ActivityId, EntityRef, ParticipantId, ResourceCostGoal, ResourceId, RoomChangeGoal, ScoreLevel};

let problem = problem
    .with_resource_cost_goal(ResourceCostGoal {
        category: "class 5a in its home room".to_string(),
        level: ScoreLevel::Weak,
        weight: 1,
        activities: vec![ActivityId(1), ActivityId(2)],
        costs: vec![(ResourceId(10), 0)],
        default_cost: 3,
    })
    .with_room_change_goal(RoomChangeGoal {
        category: "Ana's room changes".to_string(),
        level: ScoreLevel::Weak,
        entity: EntityRef::Participant(ParticipantId(1)),
        locations: vec![(ResourceId(10), 0), (ResourceId(20), 1)], // main building, annex
        travel_gap: 1,
        change_weight: 1,
        travel_weight: 5,
    });
```

## Presence and free time of several entities

These three goals have no constructor; build them as struct literals with the fields below plus
`category`, `level` and `weight`.

| Goal | Builder | Fields | Every counted unit is |
| --- | --- | --- | --- |
| `IsolatedBlockGoal` | `with_isolated_block_goal` | `entity`, `split_offset`, `max_units` | a half of an active bucket (split at `split_offset`) holding something but at most `max_units` — "nobody comes in for a single afternoon period" |
| `JointPresenceGoal` | `with_joint_presence_goal` | `anchor`, `companion` | a bucket in which `anchor` is active without sharing an activity with `companion` — "the class teacher sees the class every day" |
| `ConcurrentFreeGoal` | `with_concurrent_free_goal` | `entities`, `start_offset`, `end_offset`, `limit` | an entity over `limit` free at the same unit inside `[start_offset, end_offset)` of their active buckets — "not every class at lunch at once" |

The entities of these goals must be known resources or participants; anything else is a compile
error.
