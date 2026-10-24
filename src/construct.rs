//! Greedy construction over **activities** instead of over variables (timbra plan/61).
//!
//! The construction in `unifier` assigns variable by variable: first a start, at some point later
//! the resource choice of the same activity. On 580 activities it did not get near a feasible
//! schedule that way, while a greedy placement that decides start, resources and participants
//! **together** per activity found one in milliseconds. This module is that placement,
//! domain-free: it knows activities, participants, resources and the relations between them,
//! nothing beyond that.
//!
//! The result is a **starting assignment**, not a result. It need not satisfy every hard rule —
//! it does not know minimum distances, daily loads and no-overlap relations, and whatever could
//! not be placed lies somewhere. The repair in `unifier` (`LocalSearchSolver::repair_from`) builds
//! on it, and only the result of the repair goes through the usual check.

use crate::batch::{InternalCompiled, load_bucket_ranges};
use crate::model::{
    ActivityId, ActivityRelation, EntityRef, ParticipantId, ResourceId, SchedulingProblem,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::time::Instant;
use unifier::constraint::PropagationResult;
use unifier::{BucketRange, PropagationEngine, TrailedDomains, VariableId};

/// How many restarts at most. A failed attempt moves the unit that failed to the front for the
/// next one — the order learns, the randomness spreads.
const ATTEMPTS: u32 = 60;

/// Above this horizon length the construction gives up: one occupancy array per holder would be
/// too large, and such fine time grids are not what it is built for.
const MAX_HORIZON: i64 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Holder {
    Participant(ParticipantId),
    Resource(ResourceId),
}

/// Activities whose starts are rigidly tied together (same start, fixed offset) are placed as
/// one unit: `offset` is the distance to the anchor's start.
struct Unit {
    members: Vec<(ActivityId, i64)>,
    /// The anchor starts at which every member gets an allowed start.
    starts: Vec<i64>,
}

/// What the placement decided for an activity.
#[derive(Clone)]
struct Placement {
    start: i64,
    /// The chosen resource per requirement slot.
    resources: Vec<ResourceId>,
    participant: Option<ParticipantId>,
}

/// Deterministic randomness, the same generator as in `unifier` — one seed, one result.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    fn mix<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = (self.next() as usize) % (index + 1);
            items.swap(index, other);
        }
    }
}

/// Occupancy per holder as a counter per time unit across the horizon.
struct Occupancy {
    origin: i64,
    length: usize,
    counts: HashMap<Holder, Vec<u16>>,
}

impl Occupancy {
    fn new(origin: i64, length: usize) -> Self {
        Self {
            origin,
            length,
            counts: HashMap::new(),
        }
    }

    fn range(&self, start: i64, end: i64) -> std::ops::Range<usize> {
        let from = (start - self.origin).clamp(0, self.length as i64) as usize;
        let to = (end - self.origin).clamp(0, self.length as i64) as usize;
        from..to
    }

    fn free(&self, holder: Holder, capacity: u16, start: i64, end: i64) -> bool {
        match self.counts.get(&holder) {
            None => capacity > 0,
            Some(slots) => slots[self.range(start, end)]
                .iter()
                .all(|&count| count < capacity),
        }
    }

    fn take(&mut self, holder: Holder, start: i64, end: i64) {
        let range = self.range(start, end);
        let length = self.length;
        let slots = self.counts.entry(holder).or_insert_with(|| vec![0; length]);
        for count in &mut slots[range] {
            *count = count.saturating_add(1);
        }
    }
}

impl InternalCompiled {
    /// A complete starting assignment of all start, end and selection variables that places the
    /// activities with as few conflicts as possible — or `None` where the construction does not
    /// apply (empty domain after propagation, horizon too long, no activities).
    pub(crate) fn greedy_start(
        &self,
        problem: &SchedulingProblem,
        deadline: Option<Instant>,
    ) -> Option<HashMap<VariableId, i64>> {
        if self.activities.is_empty() {
            return None;
        }
        let mut domains = TrailedDomains::new(self.graph.domains().clone());
        if PropagationEngine::new().propagate(&self.graph, &mut domains, None)
            == PropagationResult::Conflict
        {
            return None;
        }

        // Allowed starts per activity: whatever propagation left, so including calendar
        // closures, breaks and day boundaries.
        let mut allowed: BTreeMap<ActivityId, Vec<i64>> = BTreeMap::new();
        for (&activity, &(start, _)) in &self.variables {
            let values = domains.get(&start)?.values();
            if values.is_empty() {
                return None;
            }
            allowed.insert(activity, values);
        }
        let origin = self
            .activities
            .values()
            .map(|activity| activity.allowed_window().start)
            .min()?;
        let horizon_end = self
            .activities
            .values()
            .map(|activity| activity.allowed_window().end)
            .max()?;
        if horizon_end - origin > MAX_HORIZON {
            return None;
        }
        let length = usize::try_from(horizon_end - origin).ok()?;

        let units = self.units(problem, &allowed);
        let capacities: HashMap<ResourceId, u16> = problem
            .resources
            .iter()
            .map(|resource| {
                (
                    resource.id(),
                    u16::try_from(resource.capacity()).unwrap_or(u16::MAX),
                )
            })
            .collect();
        let choice_group: HashMap<ActivityId, usize> = problem
            .participant_choice_groups
            .iter()
            .enumerate()
            .flat_map(|(index, group)| group.activities.iter().map(move |&id| (id, index)))
            .collect();
        // Two blocks of the same group that touch read as one longer block. Avoiding that is
        // only right where no allowed shape requires a block longer than the activities
        // themselves — otherwise exactly that merging is required.
        let pattern_group: HashMap<ActivityId, usize> = problem
            .bucket_load_patterns
            .iter()
            .enumerate()
            .filter(|(_, pattern)| pattern.is_active())
            .filter(|(_, pattern)| {
                let longest_activity = pattern
                    .activities
                    .iter()
                    .filter_map(|id| self.activities.get(id))
                    .map(|activity| activity.duration())
                    .max()
                    .unwrap_or(0);
                pattern
                    .allowed
                    .iter()
                    .flatten()
                    .all(|&block| block <= longest_activity)
            })
            .flat_map(|(index, pattern)| pattern.activities.iter().map(move |&id| (id, index)))
            .collect();
        // Per activity the start upper bounds it counts towards (index, bound).
        let mut start_limits: HashMap<ActivityId, Vec<(usize, u64)>> = HashMap::new();
        for (index, rule) in problem.maximum_bucket_starts.iter().enumerate() {
            for &activity in &rule.activities {
                start_limits
                    .entry(activity)
                    .or_default()
                    .push((index, rule.limit));
            }
        }
        // Holders that may only be active in a few buckets.
        let active_limits: HashMap<Holder, usize> = problem
            .maximum_active_buckets
            .iter()
            .filter_map(|rule| {
                let holder = match rule.entity {
                    EntityRef::Participant(participant) => Holder::Participant(participant),
                    EntityRef::Resource(resource) => Holder::Resource(resource),
                    // An activity is never "active on days" — the rule speaks about holders.
                    EntityRef::Activity(_) => return None,
                };
                Some((holder, usize::try_from(rule.limit).unwrap_or(usize::MAX)))
            })
            .collect();
        let activity_list: Vec<_> = self.activities.values().cloned().collect();
        let buckets = load_bucket_ranges(problem, &activity_list);

        // Tightest unit first: little start choice, many members, long duration.
        let mut order: Vec<usize> = (0..units.len()).collect();
        let mut rng = Lcg(0x5eed_c0de);
        rng.mix(&mut order);
        order.sort_by_key(|&index| {
            let unit = &units[index];
            let duration: u64 = unit
                .members
                .iter()
                .map(|(id, _)| self.activities[id].duration())
                .sum();
            (
                unit.starts.len(),
                std::cmp::Reverse(unit.members.len()),
                std::cmp::Reverse(duration),
            )
        });

        let mut best: Option<(usize, BTreeMap<ActivityId, Placement>)> = None;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            let mut rng = Lcg(0x5eed_c0de ^ u64::from(attempt).wrapping_mul(0x9e37_79b9));
            let mut state = Attempt {
                occupancy: Occupancy::new(origin, length),
                pattern_blocks: HashMap::new(),
                starts: HashMap::new(),
                active: HashMap::new(),
                chosen: HashMap::new(),
                load: HashMap::new(),
                placements: BTreeMap::new(),
            };
            let mut failures = 0usize;
            let mut first_failure: Option<usize> = None;
            for &index in &order {
                let unit = &units[index];
                let mut starts = unit.starts.clone();
                rng.mix(&mut starts);
                let placed = starts.iter().find_map(|&anchor| {
                    self.try_place(
                        unit,
                        anchor,
                        &state,
                        &Rules {
                            capacities: &capacities,
                            choice_group: &choice_group,
                            pattern_group: &pattern_group,
                            start_limits: &start_limits,
                            active_limits: &active_limits,
                            buckets: &buckets,
                        },
                        &mut rng,
                    )
                });
                match placed {
                    Some(placements) => self.commit(
                        placements,
                        &mut state,
                        &Rules {
                            capacities: &capacities,
                            choice_group: &choice_group,
                            pattern_group: &pattern_group,
                            start_limits: &start_limits,
                            active_limits: &active_limits,
                            buckets: &buckets,
                        },
                    ),
                    None => {
                        failures += 1;
                        first_failure.get_or_insert(index);
                        // Put it somewhere, but do not occupy: the repair moves it, and the
                        // remaining units are not displaced by it.
                        let anchor = starts[0];
                        for &(activity, offset) in &unit.members {
                            let placement = self.fallback_placement(activity, anchor + offset);
                            state.placements.insert(activity, placement);
                        }
                    }
                }
            }
            if best.as_ref().is_none_or(|(held, _)| failures < *held) {
                best = Some((failures, state.placements));
            }
            if failures == 0 {
                break;
            }
            if let Some(failed) = first_failure {
                order.retain(|&index| index != failed);
                order.insert(0, failed);
            }
        }

        let (_, placements) = best?;
        Some(self.to_assignment(&placements))
    }

    /// Joins activities via `SameStart` and `FixedOffset` into units with fixed offsets.
    /// Hard relations only: the search may also give up a preferred simultaneity.
    fn units(
        &self,
        problem: &SchedulingProblem,
        allowed: &BTreeMap<ActivityId, Vec<i64>>,
    ) -> Vec<Unit> {
        let mut edges: HashMap<ActivityId, Vec<(ActivityId, i64)>> = HashMap::new();
        for relation in problem.relations.iter().filter(|r| r.penalty.is_none()) {
            let offset = match relation.relation {
                ActivityRelation::SameStart => 0,
                ActivityRelation::FixedOffset { offset } => offset,
                _ => continue,
            };
            if !allowed.contains_key(&relation.first) || !allowed.contains_key(&relation.second) {
                continue;
            }
            edges
                .entry(relation.first)
                .or_default()
                .push((relation.second, offset));
            edges
                .entry(relation.second)
                .or_default()
                .push((relation.first, -offset));
        }
        let mut seen: HashSet<ActivityId> = HashSet::new();
        let mut units = Vec::new();
        for &root in allowed.keys() {
            if !seen.insert(root) {
                continue;
            }
            let mut members = vec![(root, 0i64)];
            let mut queue = VecDeque::from([(root, 0i64)]);
            while let Some((current, offset)) = queue.pop_front() {
                for &(next, delta) in edges.get(&current).into_iter().flatten() {
                    if seen.insert(next) {
                        members.push((next, offset + delta));
                        queue.push_back((next, offset + delta));
                    }
                }
            }
            let member_sets: Vec<(HashSet<i64>, i64)> = members
                .iter()
                .map(|(id, offset)| (allowed[id].iter().copied().collect(), *offset))
                .collect();
            let starts = allowed[&root]
                .iter()
                .copied()
                .filter(|&anchor| {
                    member_sets
                        .iter()
                        .all(|(set, offset)| set.contains(&(anchor + offset)))
                })
                .collect::<Vec<_>>();
            // Without a common start the unit can still be placed — just not conflict-free.
            let starts = if starts.is_empty() {
                allowed[&root].clone()
            } else {
                starts
            };
            units.push(Unit { members, starts });
        }
        units
    }

    /// The fixed holders of an activity: participants (including those from resolved groups),
    /// required participants and required resources.
    fn fixed_holders(&self, activity: ActivityId) -> Vec<Holder> {
        let mut holders: Vec<Holder> = self.activities[&activity]
            .participants()
            .iter()
            .map(|&participant| Holder::Participant(participant))
            .collect();
        holders.extend(
            self.fixed_participants
                .get(&activity)
                .into_iter()
                .flatten()
                .map(|&participant| Holder::Participant(participant)),
        );
        holders.extend(
            self.fixed_resources
                .get(&activity)
                .into_iter()
                .flatten()
                .map(|&resource| Holder::Resource(resource)),
        );
        holders.sort_unstable();
        holders.dedup();
        holders
    }

    fn try_place(
        &self,
        unit: &Unit,
        anchor: i64,
        state: &Attempt,
        rules: &Rules<'_>,
        rng: &mut Lcg,
    ) -> Option<Vec<(ActivityId, Placement)>> {
        let Rules {
            capacities,
            choice_group,
            pattern_group,
            start_limits,
            active_limits,
            buckets,
        } = *rules;
        // Whether a holder may still become active in this bucket: already active, or room left.
        let mut opened: HashMap<Holder, Vec<usize>> = HashMap::new();
        let may_be_active =
            |opened: &HashMap<Holder, Vec<usize>>, holder: Holder, bucket: usize| {
                let Some(&limit) = active_limits.get(&holder) else {
                    return true;
                };
                let used = state.active.get(&holder);
                if used.is_some_and(|set| set.contains(&bucket))
                    || opened.get(&holder).is_some_and(|new| new.contains(&bucket))
                {
                    return true;
                }
                used.map_or(0, BTreeSet::len) + opened.get(&holder).map_or(0, Vec::len) < limit
            };
        // Starts this unit itself already claims per (rule, bucket).
        let mut own_starts: HashMap<(usize, usize), u64> = HashMap::new();
        // What this unit itself already occupies: two members may not share a holder.
        let mut own: Vec<(Holder, i64, i64)> = Vec::new();
        let taken_by_unit = |own: &[(Holder, i64, i64)], holder: Holder, start: i64, end: i64| {
            own.iter()
                .any(|&(other, from, to)| other == holder && from < end && start < to)
        };
        let capacity = |holder: Holder| match holder {
            Holder::Participant(_) => 1,
            Holder::Resource(resource) => capacities.get(&resource).copied().unwrap_or(1),
        };
        let mut result = Vec::with_capacity(unit.members.len());
        for &(activity, offset) in &unit.members {
            let start = anchor + offset;
            let end = start + i64::try_from(self.activities[&activity].duration()).ok()?;
            let bucket = bucket_of(buckets, start);
            for holder in self.fixed_holders(activity) {
                if !state.occupancy.free(holder, capacity(holder), start, end)
                    || taken_by_unit(&own, holder, start, end)
                    || !may_be_active(&opened, holder, bucket)
                {
                    return None;
                }
                own.push((holder, start, end));
                opened.entry(holder).or_default().push(bucket);
            }
            for &(rule, limit) in start_limits.get(&activity).into_iter().flatten() {
                let key = (rule, bucket_of(buckets, start));
                let used = state.starts.get(&key).copied().unwrap_or(0)
                    + own_starts.get(&key).copied().unwrap_or(0);
                if used >= limit {
                    return None;
                }
                *own_starts.entry(key).or_default() += 1;
            }
            if let Some(&pattern) = pattern_group.get(&activity) {
                let bucket = bucket_of(buckets, start);
                let touches = state
                    .pattern_blocks
                    .get(&(pattern, bucket))
                    .into_iter()
                    .flatten()
                    .any(|&(from, to)| start <= to && from <= end);
                if touches {
                    return None;
                }
            }
            let mut resources = Vec::new();
            for slot in self.selected_resources.get(&activity).into_iter().flatten() {
                let mut candidates: Vec<ResourceId> =
                    slot.iter().map(|&(resource, _)| resource).collect();
                rng.mix(&mut candidates);
                let resource = candidates.into_iter().find(|&resource| {
                    let holder = Holder::Resource(resource);
                    state.occupancy.free(holder, capacity(holder), start, end)
                        && !taken_by_unit(&own, holder, start, end)
                        && may_be_active(&opened, holder, bucket)
                })?;
                own.push((Holder::Resource(resource), start, end));
                opened
                    .entry(Holder::Resource(resource))
                    .or_default()
                    .push(bucket);
                resources.push(resource);
            }
            let mut participant = None;
            if let Some(candidates) = self.selected_participants.get(&activity) {
                let decided = choice_group
                    .get(&activity)
                    .and_then(|group| state.chosen.get(group));
                let mut options: Vec<ParticipantId> = match decided {
                    Some(&chosen) => vec![chosen],
                    None => candidates
                        .iter()
                        .map(|&(participant, _)| participant)
                        .collect(),
                };
                rng.mix(&mut options);
                options.sort_by_key(|option| state.load.get(option).copied().unwrap_or(0));
                let picked = options.into_iter().find(|&option| {
                    let holder = Holder::Participant(option);
                    state.occupancy.free(holder, 1, start, end)
                        && !taken_by_unit(&own, holder, start, end)
                        && may_be_active(&opened, holder, bucket)
                })?;
                own.push((Holder::Participant(picked), start, end));
                opened
                    .entry(Holder::Participant(picked))
                    .or_default()
                    .push(bucket);
                participant = Some(picked);
            }
            result.push((
                activity,
                Placement {
                    start,
                    resources,
                    participant,
                },
            ));
        }
        Some(result)
    }

    fn commit(
        &self,
        placements: Vec<(ActivityId, Placement)>,
        state: &mut Attempt,
        rules: &Rules<'_>,
    ) {
        let Rules {
            choice_group,
            pattern_group,
            start_limits,
            active_limits,
            buckets,
            ..
        } = *rules;
        for (activity, placement) in placements {
            let duration = i64::try_from(self.activities[&activity].duration()).unwrap_or(0);
            let (start, end) = (placement.start, placement.start + duration);
            let bucket = bucket_of(buckets, start);
            let mut holders = self.fixed_holders(activity);
            holders.extend(placement.resources.iter().map(|&r| Holder::Resource(r)));
            holders.extend(placement.participant.map(Holder::Participant));
            for &holder in &holders {
                if active_limits.contains_key(&holder) {
                    state.active.entry(holder).or_default().insert(bucket);
                }
            }
            for holder in self.fixed_holders(activity) {
                state.occupancy.take(holder, start, end);
            }
            for &resource in &placement.resources {
                state.occupancy.take(Holder::Resource(resource), start, end);
            }
            if let Some(participant) = placement.participant {
                state
                    .occupancy
                    .take(Holder::Participant(participant), start, end);
                *state.load.entry(participant).or_default() += duration;
                if let Some(&group) = choice_group.get(&activity) {
                    state.chosen.entry(group).or_insert(participant);
                }
            }
            for &(rule, _) in start_limits.get(&activity).into_iter().flatten() {
                *state
                    .starts
                    .entry((rule, bucket_of(buckets, start)))
                    .or_default() += 1;
            }
            if let Some(&pattern) = pattern_group.get(&activity) {
                state
                    .pattern_blocks
                    .entry((pattern, bucket_of(buckets, start)))
                    .or_default()
                    .push((start, end));
            }
            state.placements.insert(activity, placement);
        }
    }

    /// A placement without regard to conflicts: first candidate per slot.
    fn fallback_placement(&self, activity: ActivityId, start: i64) -> Placement {
        Placement {
            start,
            resources: self
                .selected_resources
                .get(&activity)
                .into_iter()
                .flatten()
                .filter_map(|slot| slot.first().map(|&(resource, _)| resource))
                .collect(),
            participant: self
                .selected_participants
                .get(&activity)
                .and_then(|candidates| candidates.first().map(|&(participant, _)| participant)),
        }
    }

    /// Translates the placements into values for start, end and selection variables. Whatever
    /// is missing here, `repair_from` sets itself.
    fn to_assignment(
        &self,
        placements: &BTreeMap<ActivityId, Placement>,
    ) -> HashMap<VariableId, i64> {
        let mut assignment = HashMap::new();
        for (activity, placement) in placements {
            let Some(&(start, end)) = self.variables.get(activity) else {
                continue;
            };
            let duration = i64::try_from(self.activities[activity].duration()).unwrap_or(0);
            assignment.insert(start, placement.start);
            assignment.insert(end, placement.start + duration);
            for (slot, chosen) in self
                .selected_resources
                .get(activity)
                .into_iter()
                .flatten()
                .zip(&placement.resources)
            {
                for &(resource, presence) in slot {
                    assignment.insert(presence, i64::from(resource == *chosen));
                }
            }
            if let Some(chosen) = placement.participant {
                for &(participant, presence) in self
                    .selected_participants
                    .get(activity)
                    .into_iter()
                    .flatten()
                {
                    assignment.insert(presence, i64::from(participant == chosen));
                }
            }
        }
        assignment
    }
}

/// The state of one attempt.
struct Attempt {
    occupancy: Occupancy,
    /// Per (block-shape group, bucket) the placed blocks — two blocks of the same group may not
    /// touch within a bucket, otherwise they would read as one longer block.
    pattern_blocks: HashMap<(usize, usize), Vec<(i64, i64)>>,
    /// Per (start upper bound, bucket), how many activities already start there.
    starts: HashMap<(usize, usize), u64>,
    /// Per holder with a bucket limit, the buckets in which it is already active.
    active: HashMap<Holder, BTreeSet<usize>>,
    /// The chosen participant per choice group.
    chosen: HashMap<usize, ParticipantId>,
    /// Occupied time per selectable participant, so that the choice spreads out.
    load: HashMap<ParticipantId, i64>,
    placements: BTreeMap<ActivityId, Placement>,
}

/// The rules an attempt observes while placing — bundled, because every placement needs all of
/// them.
#[derive(Clone, Copy)]
struct Rules<'a> {
    capacities: &'a HashMap<ResourceId, u16>,
    choice_group: &'a HashMap<ActivityId, usize>,
    pattern_group: &'a HashMap<ActivityId, usize>,
    start_limits: &'a HashMap<ActivityId, Vec<(usize, u64)>>,
    active_limits: &'a HashMap<Holder, usize>,
    buckets: &'a [BucketRange],
}

fn bucket_of(buckets: &[BucketRange], value: i64) -> usize {
    buckets
        .iter()
        .find(|range| range.start <= value && value < range.end)
        .map_or(usize::MAX, |range| range.bucket)
}
