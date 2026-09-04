use schedulr::{
    ConflictSeverity, Participant, ParticipantId, ProposedActivity, Resource, ResourceId,
    ResourceRequirement, SchedulingState, TimeWindow,
};

fn state() -> SchedulingState {
    SchedulingState::new(
        [Resource::new(ResourceId(1), "room", 1)],
        [
            Participant::new(ParticipantId(1), "Alex"),
            Participant::new(ParticipantId(2), "Blair"),
        ],
    )
}

fn appointment(start: i64, end: i64) -> ProposedActivity {
    ProposedActivity::new("appointment", TimeWindow::new(start, end))
        .with_requirement(ResourceRequirement::new(ResourceId(1), 1))
        .with_participant(ParticipantId(1))
}

#[test]
fn create_checks_and_commits_one_activity() {
    let mut state = state();
    let first = state.commit(appointment(10, 20)).expect("first slot fits");
    assert_eq!(state.len(), 1);

    let conflicts = state.check_feasibility(&appointment(15, 25));
    assert!(conflicts.iter().any(|conflict| {
        conflict.severity == ConflictSeverity::Blocking && conflict.involved.contains(&first)
    }));
}

#[test]
fn move_excludes_the_activity_old_assignment() {
    let mut state = state();
    let activity = state.commit(appointment(10, 20)).expect("first slot fits");
    let moved = appointment(15, 25).excluding(activity);
    assert!(state.check_feasibility(&moved).is_empty());
    assert_eq!(state.commit(moved).expect("move fits"), activity);
    assert_eq!(
        state.assignment(activity).unwrap().window,
        TimeWindow::new(15, 25)
    );
}

#[test]
fn cancel_removes_the_activity_without_solving() {
    let mut state = state();
    let activity = state.commit(appointment(10, 20)).expect("first slot fits");
    assert!(state.cancel(activity).is_some());
    assert!(state.is_empty());
    assert!(state.check_feasibility(&appointment(10, 20)).is_empty());
}

#[test]
fn adding_participant_reports_an_advisory_overlap() {
    let mut state = state();
    state
        .commit(
            ProposedActivity::new("other", TimeWindow::new(10, 20))
                .with_participant(ParticipantId(2)),
        )
        .expect("first participant is free");
    let activity = state
        .commit(appointment(10, 20).with_participant(ParticipantId(1)))
        .expect("different participant and unused room fit");

    let mut changed = state.proposal_for(activity).unwrap();
    changed.add_participant(ParticipantId(2));
    let conflicts = state.check_feasibility(&changed);
    assert!(conflicts.iter().any(|conflict| {
        conflict.severity == ConflictSeverity::Advisory && conflict.message.contains("Blair")
    }));
    assert_eq!(
        state.commit(changed).expect("advisories do not block"),
        activity
    );
}
