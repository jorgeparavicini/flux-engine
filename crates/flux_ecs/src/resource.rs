use crate::system::parameter::SystemParam;
use crate::world::World;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::Deref;

pub trait Resource: 'static {}

pub struct Resources {
    // TODO: Use component id, but it can't be called `ComponentId` as its for components and resources
    data: HashMap<TypeId, Box<dyn Any>>,
}

impl Resources {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn insert<T: Resource>(&mut self, value: T) {
        self.data.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub fn get<T: Resource>(&self) -> Option<&T> {
        self.data
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref())
    }

    pub fn get_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.data
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut())
    }

    pub fn remove<T: Resource>(&mut self) -> Option<T> {
        self.data
            .remove(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast().ok())
            .map(|boxed| *boxed)
    }
}

// TODO: This is more related to a query than a resource
#[derive(Debug)]
pub struct Res<'world, T: Resource> {
    resource: &'world T,
    _phantom: PhantomData<&'world T>,
}

impl<'world, T: Resource> Res<'world, T> {
    pub fn new(resource: &'world T) -> Self {
        Res {
            resource,
            _phantom: PhantomData,
        }
    }
}

impl<T: Resource> Deref for Res<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.resource
    }
}

impl<T: Resource> SystemParam for Res<'_, T> {
    type State = ();

    type Item<'world, 'state> = Res<'world, T>;

    fn init_state(_: &mut World) -> Self::State {
        // No state needed for resources
    }

    fn get_param<'world, 'state>(
        _state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state> {
        let resource = world.get_resource::<T>().expect("resource not found");
        Res::new(resource)
    }
}

impl<T: Resource> SystemParam for Option<Res<'_, T>> {
    type State = ();

    type Item<'world, 'state> = Option<Res<'world, T>>;

    fn init_state(_: &mut World) -> Self::State {
        // No state needed for resources
    }

    fn get_param<'world, 'state>(
        _state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state> {
        world.get_resource::<T>().map(Res::new)
    }
}
