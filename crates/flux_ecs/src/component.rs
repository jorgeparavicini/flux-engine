use std::any::TypeId;
use std::collections::HashMap;

pub trait Component: 'static {}

pub(crate) type ComponentId = usize;

#[derive(Debug, Clone)]
pub(crate) struct ComponentInfo {
    pub id: ComponentId,
    pub type_id: TypeId,
    pub size: usize,
    pub align: usize,
}

pub(crate) struct ComponentRegistry {
    type_to_id: HashMap<TypeId, ComponentId>,
    components: Vec<ComponentInfo>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            type_to_id: HashMap::new(),
            components: Vec::new(),
        }
    }

    pub fn register<T: Component>(&mut self) -> ComponentId {
        let type_id = TypeId::of::<T>();
        if let Some(&id) = self.type_to_id.get(&type_id) {
            return id;
        }

        let id = self.components.len() as ComponentId;
        let size = size_of::<T>();
        let align = align_of::<T>();

        let info = ComponentInfo {
            id,
            type_id,
            size,
            align,
        };

        self.type_to_id.insert(type_id, id);
        self.components.push(info);
        id
    }

    pub fn get_id<T: Component>(&self) -> Option<ComponentId> {
        let type_id = TypeId::of::<T>();
        self.type_to_id.get(&type_id).copied()
    }

    pub fn get_info(&self, id: ComponentId) -> Option<&ComponentInfo> {
        self.components.get(id as usize)
    }
}
