---
title: Day and bucket rules
sidebarLabel: Day and bucket rules
description: Rules on how much one person or resource does per day, how a day is shaped, and how a group of activities is spread over the days. Each is hard by default and becomes a preference with soft().
order: 3
---

Calendars decide *when* an activity may start. The rules on this page decide what a day may look
like: how long, how full, how many lessons of one course, how many different subjects. They are
added to the problem with `with_*` builders like everything else.

## Buckets

schedulr does not know what a day is. These rules count per *period bucket*, and the buckets come
from the `ScheduleTemplate` (attached with `with_calendar` or `with_schedule_template`):

- every `DayTemplate` is a bucket, reaching from its `day_offset` to the next day's offset (the
  last day reaches to the first day of the next cycle);
- the buckets repeat every `cycle_length`;
- without a schedule template the whole horizon is one bucket.

`bucket_windows(template, min_value, max_value)` returns the same buckets the solver uses, as
`BucketWindow { bucket, window }`. `bucket` is the index of the day within the cycle, so Monday
has the same index in every week. Use it when your application reasons about days itself, so it
cannot drift from the solver's notion of a day.

Where a rule names an *entity*, it is an `EntityRef::Resource` or `EntityRef::Participant`. A
resource counts the activities that require it (weighted by the requirement's units, where a
load is measured), a participant the activities it attends. A `MaximumDailyLoad`, `MinimumBreak`
or `DayRule` that names an unknown entity, or an activity, is a compile error.

## Hard or soft

Every rule on this page except `ConsecutiveRunLimit` is a hard rule: the solver never returns a
plan that breaks it. `.soft(SoftPenalty::new(category, level, weight))` turns it into a preference
instead. The rule's own violation count (units over a cap, starts over a limit, distance to an
allowed shape) is multiplied by `weight` and reported under `category` at `level` in
`score_components`, like any other goal. Activity relations take the same `.soft(...)`.

```rust
use schedulr::{MaximumDailyLoad, ParticipantId, ScoreLevel, SoftPenalty};

// At most six hours a day for Ana; every hour beyond that costs 10 on the Strong level.
let problem = problem.with_maximum_daily_load(
    MaximumDailyLoad::for_participant(ParticipantId(1), 6)
        .soft(SoftPenalty::new("Ana's long days", ScoreLevel::Strong, 10)),
);
```

A hard rule that cannot be met makes the problem infeasible; the soft form solves and reports the
cost. Use the soft form when "as few long days as possible" is what you mean.

## Load and breaks per entity

| Rule | Builder | Meaning |
| --- | --- | --- |
| `MaximumDailyLoad` | `with_maximum_daily_load` | At most `limit` occupied units per bucket |
| `MinimumBreak` | `with_minimum_break` | At least `min_distance` between the starts of any two of the entity's activities |
| `MaximumActiveBuckets` | `with_maximum_active_buckets` | Active (anything at all) in at most `limit` buckets — "on at most four days" |

`MaximumDailyLoad` and `MinimumBreak` have `for_resource` and `for_participant` constructors,
`MaximumActiveBuckets` has `for_participant`; all three have `new(entity, …)`.

`MinimumBreak` measures between start values. For a real free gap after a one-hour activity, add
the hour: a 30-minute break after a 60-minute session is `min_distance: 90` in minutes. Unlike a
`BreakTemplate`, which blocks a fixed window of the calendar, it constrains the relative placement
of one entity's activities.

`MaximumActiveBuckets` leaves the choice of free days to the search; to fix *which* day is free,
block it with `with_blocked_window` (see [Calendars and availability](../calendars/)).

## The shape of a day

A `DayRule` measures one entity's occupancy inside each concrete bucket window — Monday of week A
and Monday of week B are two days here. A bucket is *active* when the entity occupies anything in
it; offsets count from the start of each bucket window.

| `DayRuleKind` | Meaning |
| --- | --- |
| `MinimumLoad { limit }` | At least `limit` occupied units in every active bucket — "no day for a single lesson" |
| `MaximumSpan { limit }` | At most `limit` units from the first start to the last end, free time included |
| `MinimumGapBetween { limit }` | At least `limit` free units between two consecutive active buckets — "no late evening followed by an early morning" |
| `FreeRunWithin { start_offset, end_offset, min_run }` | A free run of at least `min_run` inside `[start_offset, end_offset)` of every active bucket — a lunch break somewhere in the lunch window |
| `MaximumAtOffsets { start_offset, end_offset, limit }` | At most `limit` occupied units inside `[start_offset, end_offset)`, summed over all buckets — "the last period at most twice a week" |

```rust
use schedulr::{DayRule, DayRuleKind, EntityRef, ParticipantId};

// One time unit = one hour, one bucket = one day starting at 00:00.
let ben = EntityRef::Participant(ParticipantId(2));
let problem = problem
    .with_day_rule(DayRule::new(ben, DayRuleKind::MaximumSpan { limit: 8 }))
    .with_day_rule(DayRule::new(
        ben,
        DayRuleKind::FreeRunWithin { start_offset: 11, end_offset: 14, min_run: 1 },
    ));
```

In the soft form every unit a day rule misses by costs the weight.

Free time between assignments has its own preference, `IdleBounds`, described with the other
[goals](../goals/#free-time).

## Groups of activities per bucket

These rules take a list of activities, typically every occurrence of one course.

| Rule | Builder | Meaning |
| --- | --- | --- |
| `MaximumBucketStarts` | `with_maximum_bucket_starts` | At most `limit` of the activities *start* in one bucket — "at most one lesson of this course per day" |
| `MaximumDistinctPerBucket` | `with_distinct_per_bucket` | At most `limit` of the `groups` (each inner list is one group) start in one bucket — "at most five different subjects a day" |
| `BucketLoadPattern` | `with_bucket_load_pattern` | The blocks the activities form must match one of the `allowed` shapes |
| `ConsecutiveRunLimit` | `with_consecutive_run_limit` | Every unit a back-to-back run is longer than `max_run` costs `weight` (always a preference) |

`MaximumBucketStarts` counts activities, not time: a two-hour block counts once. That separates it
from `MaximumDailyLoad`.

`ConsecutiveRunLimit::new(category, level, weight, activities, max_run)` is soft only. Activities
of the list that touch or overlap form one run — "no more than two core-subject lessons in a row".
Which activities belong together is your decision; one list per participant is typical.

### Block patterns

`BucketLoadPattern::new(activities, allowed)` describes how a course is taught over the cycle.
Activities that touch or overlap inside one bucket form one block, so two adjacent one-hour
lessons are a double lesson. Each entry of `allowed` is a multiset of block lengths, order
irrelevant:

```rust
use schedulr::{ActivityId, BucketLoadPattern};

// Four hours of maths a week: a double lesson plus two singles, or four singles.
let maths: Vec<ActivityId> = (1..=4).map(ActivityId).collect();
let problem = problem.with_bucket_load_pattern(BucketLoadPattern::new(
    maths,
    vec![vec![2, 1, 1], vec![1, 1, 1, 1]],
));
```

A pattern without activities or without an allowed shape does not apply (`is_active()` is
false). In the soft form the cost is the distance to the nearest allowed shape: how many units
would have to move between blocks to reach it.

The same measurement is available outside the solver, for example to show the pattern of a
stored plan. All three functions take the activities as `(ActivityId, TimeWindow)` pairs and the
buckets as windows (the `window` of each `BucketWindow`):

| Function | Returns |
| --- | --- |
| `bucket_load_blocks(assignments, buckets)` | The block lengths per bucket, in bucket order |
| `bucket_load_pattern(assignments, buckets)` | All block lengths, largest first: `[2, 1, 1]` |
| `matches_bucket_load_pattern(pattern, assignments, buckets)` | Whether one of the allowed shapes matches; always `true` for an inactive pattern |

## Conflicts

`check`, `evaluate_move` and `explain` report broken day and bucket rules with their own
constraint names (`MaximumBucketLoad`, `BucketBlockPattern`, `MaximumActiveBuckets`,
`MinimumBreak` and the `DayRule` names); see
[Conflicts and errors](../../reference/conflicts/#constraint-names).
