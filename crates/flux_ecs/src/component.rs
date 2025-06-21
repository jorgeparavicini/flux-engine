use std::alloc::Layout;
use std::any::TypeId;
use std::collections::HashMap;
use variadics_please::all_tuples;

pub trait Component: 'static {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(pub usize);

#[derive(Debug, Clone)]
pub struct ComponentInfo {
    pub id: ComponentId,
    pub type_id: TypeId,
    pub layout: Layout,
    pub name: &'static str,
    // TODO: Add custom drop functions if needed
}

pub trait ComponentBundle {
    fn register_components(registry: &mut ComponentRegistry) -> Vec<ComponentId>;

    unsafe fn get_component_painters(&self) -> Vec<*const u8>;
}

macro_rules! impl_component_bundle_for_tuple {
    ($($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($T: Component),+> ComponentBundle for ($($T,)+) {
            fn register_components(registry: &mut ComponentRegistry) -> Vec<ComponentId> {
                vec![$(registry.register::<$T>()),+]
            }

            unsafe fn get_component_painters(&self) -> Vec<*const u8> {
                let ($($T,)+) = self;

                vec![$($T as *const $T as *const u8),+]
            }
        }
    };
}

all_tuples!(impl_component_bundle_for_tuple, 1, 16, T);

#[derive(Default)]
pub struct ComponentRegistry {
    type_to_id: HashMap<TypeId, ComponentId>,
    infos: Vec<ComponentInfo>,
}

impl ComponentRegistry {
    pub fn register<T: Component>(&mut self) -> ComponentId {
        let type_id = TypeId::of::<T>();

        *self.type_to_id.entry(type_id).or_insert_with(|| {
            let id = ComponentId(self.infos.len());
            let info = ComponentInfo {
                id,
                type_id,
                layout: Layout::new::<T>(),
                name: std::any::type_name::<T>(),
            };

            self.infos.push(info);
            id
        })
    }

    #[must_use]
    pub fn get_id<T: Component>(&self) -> Option<ComponentId> {
        let type_id = TypeId::of::<T>();
        self.type_to_id.get(&type_id).copied()
    }

    #[must_use]
    pub fn get_info(&self, id: ComponentId) -> Option<&ComponentInfo> {
        self.infos.get(id.0)
    }
}
