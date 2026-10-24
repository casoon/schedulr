//! The soft form of the hard rules (timbra plan/69, step 0): with a `SoftPenalty` an input that is
//! infeasible under the rule solves, and the counted violations are charged to the penalty's
//! category; where the rule can be met, the preference still shapes the plan.

use schedulr::{
    Activity, ActivityId, ActivityRelation, ActivityRelationConstraint, DayTemplate,
    MaximumActiveBuckets, MaximumBucketStarts, MinimumBreak, Participant, ParticipantId,
    ScheduleTemplate, SchedulingProblem, ScoreLevel, SlotTemplate, SoftPenalty, Solution,
    SolveStatus, TimeWindow, compile,
};

const CATEGORY: &str = "preference";

fn penalty(weight: i64) -> SoftPenalty {
    SoftPenalty::new(CATEGORY, ScoreLevel::Medium, weight)
}

/// Day 0 is `[0, 4)`, day 1 is `[4, 8)`, four unit slots each.
fn two_days() -> ScheduleTemplate {
    let day = |start: i64| {
        (0..4).fold(DayTemplate::new(start), |day, slot| {
            day.with_slot(SlotTemplate::new(format!("{start}-{slot}"), slot, 1))
        })
    };
    ScheduleTemplate::new(8).with_day(day(0)).with_day(day(4))
}

fn lesson(id: u64, start: i64, end: i64, participant: ParticipantId) -> Activity {
    Activity::new(
        ActivityId(id),
        format!("l{id}"),
        TimeWindow::new(start, end),
        1,
    )
    .with_participant(participant)
}

fn person() -> Participant {
    Participant::new(ParticipantId(1), "P")
}

fn solve(problem: &SchedulingProblem) -> (SolveStatus, Option<Solution>) {
    let result = compile(problem).expect("the problem compiles").solve();
    (result.status, result.solution)
}

fn charged(solution: &Solution) -> i64 {
    solution
        .score_components
        .iter()
        .filter(|component| component.category == CATEGORY)
        .map(|component| component.value)
        .sum()
}

/// Asserts the hard form is infeasible and the soft form solves with exactly `expected` charged.
fn hard_fails_soft_charges(hard: SchedulingProblem, soft: SchedulingProblem, expected: i64) {
    assert_eq!(
        solve(&hard).0,
        SolveStatus::Infeasible,
        "the rule cannot hold"
    );
    let (status, solution) = solve(&soft);
    assert_eq!(status, SolveStatus::Feasible);
    let solution = solution.expect("a preference never blocks a plan");
    assert_eq!(solution.score.hard, 0);
    assert_eq!(charged(&solution), expected);
}

#[test]
fn minimum_break_counts_every_pair_too_close() {
    let p = person();
    let base = || {
        SchedulingProblem::new(
            vec![],
            vec![p.clone()],
            (1..=3).map(|id| lesson(id, 0, 3, p.id())).collect(),
        )
    };
    let rule = MinimumBreak::for_participant(p.id(), 2);
    // Three starts in [0, 3) with distance 2: at best 0, 1, 2 — two pairs are too close.
    hard_fails_soft_charges(
        base().with_minimum_break(rule.clone()),
        base().with_minimum_break(rule.soft(penalty(1))),
        -2,
    );
}

#[test]
fn active_buckets_charge_each_bucket_over_the_limit() {
    let p = person();
    let base = || {
        SchedulingProblem::new(
            vec![],
            vec![p.clone()],
            vec![lesson(1, 0, 1, p.id()), lesson(2, 4, 5, p.id())],
        )
        .with_schedule_template(two_days())
    };
    let rule = MaximumActiveBuckets::for_participant(p.id(), 1);
    hard_fails_soft_charges(
        base().with_maximum_active_buckets(rule.clone()),
        base().with_maximum_active_buckets(rule.soft(penalty(5))),
        -5,
    );
}

#[test]
fn bucket_starts_charge_each_start_over_the_limit() {
    let p = person();
    let base = || {
        SchedulingProblem::new(
            vec![],
            vec![p.clone()],
            (1..=3).map(|id| lesson(id, 0, 4, p.id())).collect(),
        )
        .with_schedule_template(two_days())
    };
    let rule = MaximumBucketStarts::new(vec![ActivityId(1), ActivityId(2), ActivityId(3)], 1);
    hard_fails_soft_charges(
        base().with_maximum_bucket_starts(rule.clone()),
        base().with_maximum_bucket_starts(rule.soft(penalty(1))),
        -2,
    );
}

#[test]
fn a_broken_relation_costs_its_weight() {
    let p = person();
    let base = || {
        SchedulingProblem::new(
            vec![],
            vec![p.clone()],
            vec![lesson(1, 0, 1, p.id()), lesson(2, 4, 5, p.id())],
        )
    };
    let rule =
        ActivityRelationConstraint::new(ActivityId(1), ActivityId(2), ActivityRelation::SameStart);
    hard_fails_soft_charges(
        base().with_relation(rule.clone()),
        base().with_relation(rule.soft(penalty(7))),
        -7,
    );
}

/// Where the relation can hold, the preference makes it hold.
#[test]
fn a_soft_relation_is_met_when_it_can_be() {
    let p = person();
    let other = Participant::new(ParticipantId(2), "Q");
    let problem = SchedulingProblem::new(
        vec![],
        vec![p.clone(), other.clone()],
        vec![lesson(1, 0, 4, p.id()), lesson(2, 0, 4, other.id())],
    )
    .with_relation(
        ActivityRelationConstraint::new(ActivityId(1), ActivityId(2), ActivityRelation::SameStart)
            .soft(penalty(1)),
    );
    let solution = solve(&problem).1.expect("both fit");
    assert_eq!(charged(&solution), 0, "the two start together");
}
