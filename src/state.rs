use crate::batch::build_internal;
use crate::model::{
    Activity, ActivityId, Assignment, Conflict, ConflictSeverity, EntityRef, Participant,
    ParticipantId, ProposedActivity, Resource, ResourceId, ResourcePool, ResourcePoolId,
    ResourceRequirement,
};
use std::collections::{BTreeMap, BTreeSet};
use unifier::constraint::Assignment as UnifierAssignment;

/// In-memory snapshot used for synchronous single-activity feasibility checks.
#[derive(Debug, Default)]
pub struct SchedulingState {
    resources: BTreeMap<ResourceId, Resource>,
    participants: BTreeMap<ParticipantId, Participant>,
    resource_pools: BTreeMap<ResourcePoolId, ResourcePool>,
    committed: BTreeMap<ActivityId, (Activity, Assignment)>,
    activities_by_resource: BTreeMap<ResourceId, BTreeSet<ActivityId>>,
    activities_by_participant: BTreeMap<ParticipantId, BTreeSet<ActivityId>>,
    next_activity_id: u64,
}

impl SchedulingState {
    pub fn new(
        resources: impl IntoIterator<Item = Resource>,
        participants: impl IntoIterator<Item = Participant>,
        resource_pools: impl IntoIterator<Item = ResourcePool>,
    ) -> Self {
        Self {
            resources: resources
                .into_iter()
                .map(|resource| (resource.id(), resource))
                .collect(),
            participants: participants
                .into_iter()
                .map(|participant| (participant.id(), participant))
                .collect(),
            resource_pools: resource_pools
                .into_iter()
                .map(|pool| (pool.id, pool))
                .collect(),
            committed: BTreeMap::new(),
            activities_by_resource: BTreeMap::new(),
            activities_by_participant: BTreeMap::new(),
            next_activity_id: 0,
        }
    }

    /// Evaluates one create or move operation without invoking a solver.
    pub fn check_feasibility(&self, proposal: &ProposedActivity) -> Vec<Conflict> {
        let activity_id = proposal
            .excluded_activity()
            .unwrap_or(ActivityId(self.next_activity_id));
        if let Some(excluded) = proposal.excluded_activity()
            && !self.committed.contains_key(&excluded)
        {
            return vec![model_conflict(format!(
                "Cannot exclude unknown activity {excluded}"
            ))];
        }

        let Some(duration) = proposal.window().duration() else {
            return vec![model_conflict("Activity time window is invalid")];
        };
        let mut proposed_activity =
            Activity::new(activity_id, proposal.name(), proposal.window(), duration);
        for &participant in proposal.participants() {
            proposed_activity = proposed_activity.with_participant(participant);
        }
        for requirement in proposal.requirements() {
            proposed_activity = proposed_activity.with_requirement(requirement.clone());
        }

        let mut relevant = BTreeSet::new();
        for requirement in proposal.requirements() {
            for resource in self.matching_resources(requirement) {
                if let Some(activities) = self.activities_by_resource.get(&resource) {
                    relevant.extend(activities);
                }
            }
        }
        for participant in proposal.participants() {
            if let Some(activities) = self.activities_by_participant.get(participant) {
                relevant.extend(activities);
            }
        }
        if let Some(excluded) = proposal.excluded_activity() {
            relevant.remove(&excluded);
        }
        let existing: Vec<&(Activity, Assignment)> = relevant
            .iter()
            .filter_map(|activity| self.committed.get(activity))
            .collect();
        let mut activities: Vec<Activity> = existing
            .iter()
            .map(|(activity, assignment)| copy_activity_with_window(activity, assignment.window))
            .collect();
        activities.push(proposed_activity);

        let resources: Vec<Resource> = self.resources.values().cloned().collect();
        let participants: Vec<Participant> = self.participants.values().cloned().collect();
        let resource_pools: Vec<ResourcePool> = self.resource_pools.values().cloned().collect();
        let compiled = match build_internal(&resources, &participants, &activities, &resource_pools)
        {
            Ok(compiled) => compiled,
            Err(error) => {
                return error
                    .messages()
                    .iter()
                    .cloned()
                    .map(model_conflict)
                    .collect();
            }
        };

        let mut committed = UnifierAssignment::new();
        for (activity, assignment) in existing {
            let (start, end) = compiled.variables_for(activity.id());
            committed.insert(start, assignment.window.start);
            committed.insert(end, assignment.window.end);
        }
        let (start, end) = compiled.variables_for(activity_id);
        compiled.check_incremental(
            &committed,
            &[
                (start, proposal.window().start),
                (end, proposal.window().end),
            ],
        )
    }

    /// Commits a proposal when it has no blocking conflicts. Advisory conflicts do not block.
    pub fn commit(&mut self, proposal: ProposedActivity) -> Result<ActivityId, Vec<Conflict>> {
        let conflicts = self.check_feasibility(&proposal);
        if conflicts
            .iter()
            .any(|conflict| conflict.severity == ConflictSeverity::Blocking)
        {
            return Err(conflicts);
        }

        let activity_id = proposal
            .excluded_activity()
            .unwrap_or(ActivityId(self.next_activity_id));
        let duration = proposal
            .window()
            .duration()
            .expect("validated by check_feasibility");
        let mut activity = Activity::new(activity_id, proposal.name(), proposal.window(), duration);
        for &participant in proposal.participants() {
            activity = activity.with_participant(participant);
        }
        for requirement in proposal.requirements() {
            activity = activity.with_requirement(requirement.clone());
        }
        if let Some((old_activity, _)) = self.committed.remove(&activity_id) {
            self.remove_from_indexes(&old_activity);
        }
        self.add_to_indexes(&activity);
        self.committed.insert(
            activity_id,
            (activity, Assignment::new(activity_id, proposal.window())),
        );
        if proposal.excluded_activity().is_none() {
            self.next_activity_id = self.next_activity_id.saturating_add(1);
        }
        Ok(activity_id)
    }

    pub fn cancel(&mut self, activity: ActivityId) -> Option<(Activity, Assignment)> {
        let committed = self.committed.remove(&activity)?;
        self.remove_from_indexes(&committed.0);
        Some(committed)
    }

    pub fn activity(&self, activity: ActivityId) -> Option<&Activity> {
        self.committed.get(&activity).map(|(activity, _)| activity)
    }

    pub fn assignment(&self, activity: ActivityId) -> Option<&Assignment> {
        self.committed
            .get(&activity)
            .map(|(_, assignment)| assignment)
    }

    pub fn proposal_for(&self, activity: ActivityId) -> Option<ProposedActivity> {
        let (activity, assignment) = self.committed.get(&activity)?;
        let mut proposal =
            ProposedActivity::new(activity.name(), assignment.window).excluding(activity.id());
        for &participant in activity.participants() {
            proposal = proposal.with_participant(participant);
        }
        for requirement in activity.requirements() {
            proposal = proposal.with_requirement(requirement.clone());
        }
        Some(proposal)
    }

    pub fn len(&self) -> usize {
        self.committed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.committed.is_empty()
    }

    fn add_to_indexes(&mut self, activity: &Activity) {
        for requirement in activity.requirements() {
            for resource in self.matching_resources(requirement) {
                self.activities_by_resource
                    .entry(resource)
                    .or_default()
                    .insert(activity.id());
            }
        }
        for &participant in activity.participants() {
            self.activities_by_participant
                .entry(participant)
                .or_default()
                .insert(activity.id());
        }
    }

    fn remove_from_indexes(&mut self, activity: &Activity) {
        for requirement in activity.requirements() {
            for resource in self.matching_resources(requirement) {
                if let Some(activities) = self.activities_by_resource.get_mut(&resource) {
                    activities.remove(&activity.id());
                }
            }
        }
        for participant in activity.participants() {
            if let Some(activities) = self.activities_by_participant.get_mut(participant) {
                activities.remove(&activity.id());
            }
        }
    }

    fn matching_resources(&self, requirement: &ResourceRequirement) -> Vec<ResourceId> {
        self.resources
            .values()
            .filter(|resource| {
                requirement
                    .exact_resource()
                    .is_none_or(|id| id == resource.id())
                    && requirement
                        .resource_type()
                        .is_none_or(|kind| kind == resource.resource_type())
                    && resource.capacity() >= requirement.minimum_capacity()
                    && requirement
                        .required_features()
                        .is_subset(resource.features())
                    && (requirement.candidates().is_empty()
                        || requirement.candidates().contains(&resource.id()))
                    && requirement.pool().is_none_or(|pool| {
                        self.resource_pools
                            .get(&pool)
                            .is_some_and(|pool| pool.resources.contains(&resource.id()))
                    })
            })
            .map(Resource::id)
            .collect()
    }
}

fn copy_activity_with_window(activity: &Activity, window: crate::TimeWindow) -> Activity {
    let mut copy = Activity::new(activity.id(), activity.name(), window, activity.duration());
    for &participant in activity.participants() {
        copy = copy.with_participant(participant);
    }
    for requirement in activity.requirements() {
        copy = copy.with_requirement(requirement.clone());
    }
    copy
}

fn model_conflict(message: impl Into<String>) -> Conflict {
    Conflict {
        severity: ConflictSeverity::Blocking,
        constraint_name: "Model".to_string(),
        involved: Vec::new(),
        entity: None::<EntityRef>,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_requirements_are_indexed_only_under_pool_members() {
        let member = Resource::new(ResourceId(1), "member", 1);
        let outside = Resource::new(ResourceId(2), "outside", 1);
        let pool = ResourcePool::new(ResourcePoolId(1), "rooms", [member.id()]);
        let mut state = SchedulingState::new([member, outside], [], [pool]);

        state
            .commit(
                ProposedActivity::new("pooled", crate::TimeWindow::new(0, 1))
                    .with_requirement(ResourceRequirement::from_pool(ResourcePoolId(1), 1)),
            )
            .unwrap();

        assert_eq!(
            state
                .activities_by_resource
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![ResourceId(1)]
        );
    }
}
