/*
pub unsafe trait QueryParam<'a>: Sized {
    fn component_ids(registry: &ComponentRegistry) -> Vec<ComponentId>;

    unsafe fn query_archetype(
        archetype: &'a Archetype,
        component_ids: &[ComponentId],
    ) -> Box<dyn Iterator<Item = Self> + 'a>;
}

pub struct Query<'a, T: QueryParam<'a>> {
    archetypes: Vec<&'a Archetype>,
    _marker: std::marker::PhantomData<T>,
    world: &'a World,
}

unsafe impl<'a, T: QueryParam<'a>> SystemParam for Query<'a, T> {
    fn resolve(world: &World) -> Self {
        Self::new(world)
    }
}

impl<'a, T: QueryParam<'a>> Query<'a, T> {
    pub fn new(world: &'a World) -> Self {
        let needed = T::component_ids(world.component_registry());

        let archetypes = world
            .archetype_manager()
            .archetypes
            .iter()
            .filter_map(|(component_set, archetype)| {
                if needed.iter().all(|id| component_set.contains(*id)) {
                    Some(archetype)
                } else {
                    None
                }
            })
            .collect();

        Self {
            archetypes,
            _marker: std::marker::PhantomData,
            world,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = T> + 'a {
        self.archetypes.iter().flat_map(move |archetype| unsafe {
            T::query_archetype(
                *archetype,
                &T::component_ids(&self.world.component_registry()),
            )
        })
    }
}
*/
