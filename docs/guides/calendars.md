---
title: Calendars and availability
description: Periodic slot templates decide when activities may start. Absolute exceptions, breaks and per-person or per-resource unavailability narrow that further.
order: 2
---

Without a calendar, an activity may start at any value inside its allowed window. A calendar
restricts the start times of every activity in the problem.

## Periodic slot templates

A `ScheduleTemplate` describes one cycle (a day, a week, an A/B fortnight) as days with slots.
`with_calendar` attaches it together with the `AcademicPeriod`, the overall horizon:

```rust
use schedulr::{AcademicPeriod, DayTemplate, ScheduleTemplate, SlotTemplate, TimeWindow};

// One time unit = one hour. The cycle is 24 hours, so this day repeats every day.
let mut day = DayTemplate::new(0);
for (offset, length) in [(9, 3), (10, 2), (11, 1), (13, 3), (14, 2), (15, 1)] {
    day = day.with_slot(SlotTemplate::new(format!("{offset:02}:00"), offset, length));
}
let calendar = ScheduleTemplate::new(24)
    .with_day(day)
    .with_unavailable_range(37, 38); // Tuesday 13:00-14:59 is closed

let problem = problem.with_calendar(
    AcademicPeriod { window: TimeWindow::new(0, 48) },
    calendar,
);
```

How it works:

- A `SlotTemplate` has a name, an `offset` inside its day and a `duration`. An activity may start
  in a slot only if the slot is at least as long as the activity.
- `DayTemplate::new(day_offset)` places a day inside the cycle. For a weekly cycle in hours you
  would use `ScheduleTemplate::new(168)` and days at 0, 24, 48 and so on.
- The allowed starts repeat every `cycle_length` units across the academic period. Every activity
  window must lie inside the academic period, otherwise `compile` reports "lies outside the
  academic period".
- `with_unavailable_range(start, end)` removes absolute start times, inclusive on both ends:
  holidays, closures, one-off events.

An empty template (no slot at all) or a cycle length of zero or less is a compile error.

## Breaks


A `BreakTemplate` is a recurring pause inside a day. Its window is relative to the day, like a
slot offset. A start is dropped if the activity would overlap the break:

```rust
use schedulr::{BreakTemplate, DayTemplate, TimeWindow};

let day = DayTemplate::new(0).with_break(BreakTemplate {
    name: "Lunch".to_string(),
    window: TimeWindow::new(12, 13),
});
```

With lunch from 12 to 13, a two-hour activity cannot start at 11 even if an 11:00 slot exists.

## Availability of participants and resources


`Participant::with_unavailable_range(start, end)` and `Resource::with_unavailable_range(start,
end)` mark inclusive ranges in which the participant or resource cannot be booked:

```rust
use schedulr::{Participant, ParticipantId, Resource, ResourceId};

let ben = Participant::new(ParticipantId(2), "Ben").with_unavailable_range(9, 13);
let gym = Resource::new(ResourceId(1), "Gym", 1).with_unavailable_range(0, 3);
```

The range applies to the start time of the activity: an activity that uses Ben cannot start
anywhere from 9 to 13. An activity that starts at 8 and runs into the range is not rejected by
this rule, so model the range to cover every start that would overlap.

For fixed requirements the restriction always applies. For flexible requirements (pools,
matching, candidates) it applies only to the candidate the solver actually selects, so an
unavailable candidate simply pushes the solver to another one.

### Blocking a window instead of a start

An unavailable range counts the *start* of an activity, so a long activity may still run into it.
When the meaning is "not bookable in these hours" — a teacher's free day, a hall that opens later —
say so with `with_blocked_window`, which takes the half-open window the rest of the API uses:

```rust
use schedulr::{Participant, ParticipantId, TimeWindow};

// The 4th hour is `[3, 4)` — the teacher is not available in it, whatever the lesson's length.
let ben = Participant::new(ParticipantId(2), "Ben").with_blocked_window(TimeWindow::new(3, 4));
```

No activity may *occupy* any part of a blocked window. Blocking the single hour `[3, 4)` therefore
forbids a one-hour lesson at 3, and a two-hour lesson at 2 as well, because that one would run
through the block. `TimeWindow::blocked_starts(duration)` returns exactly the starts that are
ruled out (as an inclusive range, the shape `add_calendar` takes) and
`TimeWindow::occupies(start, duration)` answers the same question for one start.

For fixed requirements a blocked window always applies; for flexible ones only to the selected
candidate, exactly like an unavailable range. A window of zero length blocks nothing — half-open,
`[3, 3)` is empty.

The [relations and breaks](../../../showcase/relations-and-breaks/) showcase shows a lunch break
and Ben's availability in one solved day.
