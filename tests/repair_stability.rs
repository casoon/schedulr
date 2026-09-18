//! E0.2: a repair must keep the baseline stable — presence variables included — and must never
//! present a fallback solve as a repair (plan 31).

use schedulr::{
    Activity, ActivityId, Assignment, CancellationToken, Participant, ParticipantId,
    ParticipantPool, ParticipantPoolId, ParticipantRequirement, RepairOptions, RepairOutcome,
    Resource, ResourceId, ResourceRequirement, SchedulingProblem, Score, ScoreLevel, ScoreRule,
    Solution, SolveStatus, TimeWindow, compile,
};

fn baseline(assignments: Vec<Assignment>) -> Solution {
    Solution {
        assignments,
        score: Score::default(),
        score_components: Vec::new(),
    }
}

fn lesson(id: ActivityId, window: TimeWindow) -> Activity {
    Activity::new(id, "lesson", window, 1)
        .with_participant_requirement(ParticipantRequirement::from_pool(ParticipantPoolId(1)))
}

fn start_of(solution: &Solution, activity: ActivityId) -> i64 {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == activity)
        .expect("assignment present")
        .window
        .start
}

fn participants_of(solution: &Solution, activity: ActivityId) -> Vec<ParticipantId> {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == activity)
        .expect("assignment present")
        .participants
        .clone()
}

/// A repair that only has to shift activity 1 in time must leave activity 2 — and activity 1's
/// own participant — exactly as in the baseline. Participant availability forces the stable
/// choices, so the outcome is deterministic; without the E0.2 stability rule a free presence
/// variable could just as well swap them.
#[test]
fn a_time_only_repair_keeps_uninvolved_participants() {
    // Both lessons end up adjacent in the single room. Bea cannot attend the first slot and Alex
    // cannot attend the second, so the participants are pinned to the baseline pairing.
    let problem = SchedulingProblem::new(
        vec![],
        vec![
            Participant::new(ParticipantId(1), "Alex").with_blocked_window(TimeWindow::new(1, 2)),
            Participant::new(ParticipantId(2), "Bea").with_blocked_window(TimeWindow::new(0, 1)),
        ],
        vec![
            lesson(ActivityId(1), TimeWindow::new(0, 1)),
            lesson(ActivityId(2), TimeWindow::new(1, 2)),
        ],
    )
    .with_participant_pool(ParticipantPool::new(
        ParticipantPoolId(1),
        "teachers",
        [ParticipantId(1), ParticipantId(2)],
    ));
    let compiled = compile(&problem).unwrap();
    let baseline = baseline(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(2, 3)).with_participant(ParticipantId(1)),
        Assignment::new(ActivityId(2), TimeWindow::new(1, 2)).with_participant(ParticipantId(2)),
    ]);

    let repaired = compiled
        .repair(
            &baseline,
            &RepairOptions {
                time_limit: std::time::Duration::from_secs(1),
                destroy_fraction: 0.5,
                ..RepairOptions::default()
            },
        )
        .solution
        .expect("repair finds the forced arrangement");

    // Activity 1 was forced out of its baseline slot …
    assert_eq!(start_of(&repaired, ActivityId(1)), 0);
    // … while activity 2 kept its slot and participant, and activity 1 kept its teacher.
    assert_eq!(start_of(&repaired, ActivityId(2)), 1);
    assert_eq!(
        participants_of(&repaired, ActivityId(1)),
        vec![ParticipantId(1)]
    );
    assert_eq!(
        participants_of(&repaired, ActivityId(2)),
        vec![ParticipantId(2)]
    );
}

/// `frozen` wins over an optimisation that would otherwise move the activity: with a zero start
/// penalty the solver wants activity 1 in the preferred window, but freezing keeps it put.
#[test]
fn a_frozen_activity_is_never_moved() {
    let room = Resource::new(ResourceId(1), "A204", 1);
    let room_id = room.id();
    let activity = |id: ActivityId| {
        Activity::new(id, "lesson", TimeWindow::new(0, 3), 1)
            .with_requirement(ResourceRequirement::new(room_id, 1))
    };
    let problem = SchedulingProblem::new(
        vec![room],
        vec![],
        vec![activity(ActivityId(1)), activity(ActivityId(2))],
    )
    .with_score_rule(ScoreRule::prefer_window(
        "preferred",
        ScoreLevel::Strong,
        ActivityId(1),
        TimeWindow::new(2, 3),
        10,
    ));
    let compiled = compile(&problem).unwrap();
    let baseline = baseline(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
        Assignment::new(ActivityId(2), TimeWindow::new(1, 2)).with_resource(ResourceId(1)),
    ]);

    let repaired = compiled
        .repair(
            &baseline,
            &RepairOptions {
                // Only a small weight on time, so the preferred window (weight 10) would win —
                // unless `frozen` holds activity 1 at its start.
                change_penalty: 1,
                frozen: [ActivityId(1)].into_iter().collect(),
                time_limit: std::time::Duration::from_millis(500),
                ..RepairOptions::default()
            },
        )
        .solution
        .expect("frozen baseline is feasible");

    assert_eq!(start_of(&repaired, ActivityId(1)), 0);
}

/// A cancelled token reaches the search; a baseline that does not cover every variable makes LNS
/// hand off to the plain solver, which must abort immediately instead of producing a plan.
#[test]
fn a_cancelled_token_aborts_the_repair() {
    let room = Resource::new(ResourceId(1), "A204", 1);
    let compiled = compile(&SchedulingProblem::new(
        vec![room],
        vec![],
        vec![
            Activity::new(ActivityId(1), "first", TimeWindow::new(0, 3), 1)
                .with_requirement(ResourceRequirement::new(ResourceId(1), 1)),
            Activity::new(ActivityId(2), "second", TimeWindow::new(0, 3), 1)
                .with_requirement(ResourceRequirement::new(ResourceId(1), 1)),
        ],
    ))
    .unwrap();
    // Activity 2 has no baseline, so the baseline is incomplete.
    let incomplete = baseline(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
    ]);

    let token = CancellationToken::new();
    token.cancel();
    let result = compiled.repair(
        &incomplete,
        &RepairOptions {
            cancellation: Some(token),
            ..RepairOptions::default()
        },
    );

    assert_eq!(
        result.status,
        SolveStatus::Aborted(schedulr::AbortReason::Cancelled)
    );
    assert!(result.solution.is_none());
}

/// A baseline naming an activity the problem no longer contains makes the stability-augmented
/// problem fail to compile; that fallback must be visible, and `repair` must still return the
/// same fresh solve.
#[test]
fn a_compile_failure_falls_back_to_solve_and_reports_it() {
    let room = Resource::new(ResourceId(1), "A204", 1);
    let compiled = compile(&SchedulingProblem::new(
        vec![room],
        vec![],
        vec![
            Activity::new(ActivityId(1), "first", TimeWindow::new(0, 3), 1)
                .with_requirement(ResourceRequirement::new(ResourceId(1), 1)),
        ],
    ))
    .unwrap();
    let stale = baseline(vec![
        Assignment::new(ActivityId(1), TimeWindow::new(0, 1)).with_resource(ResourceId(1)),
        Assignment::new(ActivityId(99), TimeWindow::new(1, 2)).with_resource(ResourceId(1)),
    ]);

    let outcome = compiled.repair_with_outcome(&stale, &RepairOptions::default());

    assert_eq!(outcome.outcome, RepairOutcome::FellBackToSolve);
    assert!(outcome.result.solution.is_some());
    // The convenience wrapper drops only the signal, not the result.
    let direct = compiled.repair(&stale, &RepairOptions::default());
    assert_eq!(direct.solution, outcome.result.solution);
}
