use crate::component::ComponentKey;
use crate::entity::Entity;

/// A typed, directed link from one entity to another.
///
/// A relation carries no data of its own: the target entity is part of the
/// component id of the `(relation, target)` pair, so `(ChildOf, a)` and
/// `(ChildOf, b)` are distinct components an entity's signature can hold. An
/// entity holds at most one instance of a given relation at a time.
pub trait Relation: 'static {
    /// This relation's stable identity, shared by all of its pairs.
    const KEY: ComponentKey;
}

/// The parent link. `relate::<ChildOf>(child, parent)` records that `child`
/// is a child of `parent`; despawning `parent` despawns the whole subtree.
pub struct ChildOf;

impl Relation for ChildOf {
    const KEY: ComponentKey = ComponentKey::from_path("flux_ecs::ChildOf");
}

/// What a relation-pair component id stands for.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct RelationTarget {
    pub relation: ComponentKey,
    pub target: Entity,
}
