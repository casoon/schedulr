//! The aspects of a `Changed` assignment are exactly the fields that differ: a pure move
//! touches time only, a room swap resources only, a participant change participants only.
//! `Added`/`Removed` touch all three — the assignment as a whole appears or disappears.

use schedulr::{
    Assignment, AssignmentChange, ChangeAspects, ParticipantId, ResourceId, TimeWindow,
};

fn assignment(window: (i64, i64)) -> Assignment {
    Assignment::new(schedulr::ActivityId(1), TimeWindow::new(window.0, window.1))
}

#[test]
fn a_time_only_change_reports_time() {
    let change = AssignmentChange::Changed {
        before: assignment((0, 2)),
        after: assignment((3, 5)),
    };
    assert_eq!(
        change.aspects(),
        ChangeAspects {
            time: true,
            resources: false,
            participants: false,
        }
    );
}

#[test]
fn a_resource_only_change_reports_resources() {
    let before = assignment((0, 2)).with_resource(ResourceId(1));
    let after = assignment((0, 2)).with_resource(ResourceId(2));
    assert_eq!(
        AssignmentChange::Changed { before, after }.aspects(),
        ChangeAspects {
            time: false,
            resources: true,
            participants: false,
        }
    );
}

#[test]
fn a_participant_only_change_reports_participants() {
    let before = assignment((0, 2)).with_participant(ParticipantId(7));
    let after = assignment((0, 2)).with_participant(ParticipantId(8));
    assert_eq!(
        AssignmentChange::Changed { before, after }.aspects(),
        ChangeAspects {
            time: false,
            resources: false,
            participants: true,
        }
    );
}

#[test]
fn added_and_removed_touch_every_aspect() {
    let all = ChangeAspects {
        time: true,
        resources: true,
        participants: true,
    };
    assert_eq!(AssignmentChange::Added(assignment((0, 2))).aspects(), all);
    assert_eq!(AssignmentChange::Removed(assignment((0, 2))).aspects(), all);
}
