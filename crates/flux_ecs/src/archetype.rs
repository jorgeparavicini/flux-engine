use crate::component::{ComponentId, ComponentInfo, ComponentRegistry};
use crate::entity::Entity;
use bitvec::vec::BitVec;
use std::collections::HashMap;
use std::hash::Hash;
use std::ptr::NonNull;
use std::{
    alloc::{self, Layout},
    ops,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComponentSet {
    bits: BitVec,
}

impl ComponentSet {
    pub fn new() -> Self {
        Self {
            bits: BitVec::new(),
        }
    }

    pub fn insert(&mut self, id: ComponentId) {
        if id >= self.bits.len() {
            self.bits.resize(id + 1, false);
        }
        self.bits.set(id as usize, true);
    }

    pub fn contains(&self, id: ComponentId) -> bool {
        id < self.bits.len() && self.bits[id as usize]
    }

    pub fn union(&self, other: &Self) -> Self {
        let max_len = self.bits.len().max(other.bits.len());
        let mut new_bits = self.bits.clone();
        new_bits.resize(max_len, false);
        for i in 0..other.bits.len() {
            if other.bits[i] {
                new_bits.set(i, true);
            }
        }

        Self { bits: new_bits }
    }

    pub fn is_subset(&self, other: &Self) -> bool {
        if self.bits.len() > other.bits.len() {
            return false;
        }
        self.bits
            .iter()
            .enumerate()
            .all(|(i, b)| !b || other.bits[i])
    }
}

impl ops::BitOr for ComponentSet {
    type Output = Self;

    fn bitor(self, other: Self) -> Self::Output {
        self.union(&other)
    }
}

impl Hash for ComponentSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let bytes = self.bits.as_raw_slice();
        bytes.hash(state);
    }
}

pub(crate) struct ComponentColumn {
    ptr: NonNull<u8>,
    capacity: usize,
    length: usize,
    size: usize,
    align: usize,
}

impl ComponentColumn {
    fn with_capacity(size: usize, align: usize, capacity: usize) -> Self {
        assert!(capacity > 0, "Capacity must be greater than zero");
        let layout = Layout::from_size_align(size * capacity, align)
            .expect("Invalid layout for component column");
        let ptr = unsafe { alloc::alloc(layout) };

        if ptr.is_null() {
            alloc::handle_alloc_error(layout);
        }

        Self {
            ptr: NonNull::new(ptr).expect("Failed to create NonNull pointer"),
            capacity,
            length: 0,
            size,
            align,
        }
    }

    unsafe fn dealloc(&mut self) {
        let layout = Layout::from_size_align(self.size * self.capacity, self.align)
            .expect("Invalid layout for component column");
        unsafe {
            alloc::dealloc(self.ptr.as_ptr(), layout);
        }
        self.ptr = NonNull::dangling();
        self.capacity = 0;
        self.length = 0;
    }

    unsafe fn grow(&mut self) {
        let new_capacity = self.capacity * 2;
        let old_layout = Layout::from_size_align(self.size * self.capacity, self.align)
            .expect("Invalid layout for component column");
        let new_size = self.size * new_capacity;
        let new_layout = Layout::from_size_align(new_size, self.align)
            .expect("Invalid layout for component column");

        let new_ptr = unsafe { alloc::realloc(self.ptr.as_ptr(), old_layout, new_size) };
        if new_ptr.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        self.ptr = NonNull::new(new_ptr).expect("Failed to create NonNull pointer");
        self.capacity = new_capacity;
    }

    unsafe fn ptr_at(&self, index: usize) -> *mut u8 {
        assert!(index < self.capacity, "Index out of bounds");
        unsafe { self.ptr.as_ptr().add(index * self.size) }
    }

    unsafe fn push(&mut self, value: *const u8) {
        if self.length == self.capacity {
            unsafe {
                self.grow();
            }
        }
        unsafe {
            std::ptr::copy_nonoverlapping(value, self.ptr_at(self.length), self.size);
        }
        self.length += 1;
    }

    unsafe fn swap_remove(&mut self, index: usize) {
        assert!(index < self.length, "Index out of bounds");
        let last_index = self.length - 1;
        if index != last_index {
            unsafe {
                let src = self.ptr_at(last_index);
                let dest = self.ptr_at(index);
                std::ptr::copy_nonoverlapping(src, dest, self.size);
            }
        }
        self.length -= 1;
    }
}

impl Drop for ComponentColumn {
    fn drop(&mut self) {
        unsafe {
            self.dealloc();
        }
    }
}

pub struct Archetype {
    types: Vec<ComponentInfo>,
    columns: Vec<ComponentColumn>,
    entities: Vec<Entity>,
    component_set: ComponentSet,
}

impl Archetype {
    pub fn new(types: Vec<ComponentInfo>, initial_capacity: usize) -> Self {
        let mut component_set = ComponentSet::new();
        for type_info in &types {
            component_set.insert(type_info.id);
        }
        let columns = types
            .iter()
            .map(|ti| ComponentColumn::with_capacity(ti.size, ti.align, initial_capacity))
            .collect();

        Self {
            types,
            columns,
            entities: Vec::with_capacity(initial_capacity),
            component_set,
        }
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub unsafe fn insert(&mut self, entity: Entity, components: &[(*const u8, ComponentId)]) {
        assert_eq!(
            components.len(),
            self.types.len(),
            "Component count mismatch"
        );

        for ((ptr, cid), column) in components.iter().zip(self.columns.iter_mut()) {
            #[cfg(debug_assertions)]
            {
                let type_info = self
                    .types
                    .iter()
                    .find(|ti| ti.id == *cid)
                    .expect("ComponentId not found in types");
                assert_eq!(
                    type_info.size,
                    size_of_val(ptr),
                    "Size mismatch for the component"
                );
                assert_eq!(
                    type_info.align,
                    align_of_val(&ptr),
                    "Alignment mismatch for the component"
                );
            }

            unsafe {
                column.push(*ptr);
            }
        }

        self.entities.push(entity);
    }

    pub unsafe fn remove(&mut self, index: usize) {
        let last_index = self.entities.len() - 1;
        if index == last_index {
            for col in &mut self.columns {
                col.length -= 1;
            }
        } else {
            self.entities[index] = self.entities[last_index];
            for col in &mut self.columns {
                unsafe {
                    col.swap_remove(index);
                }
            }
        }
        self.entities.pop();
    }

    pub unsafe fn get_component_ptr(
        &self,
        component_id: ComponentId,
        index: usize,
    ) -> Option<*const u8> {
        let col_index = self.types.iter().position(|ti| ti.id == component_id)?;
        unsafe { Some(self.columns[col_index].ptr_at(index)) }
    }
}

pub(crate) struct ArchetypeManager {
    pub archetypes: HashMap<ComponentSet, Archetype>,
    entity_locations: HashMap<Entity, (ComponentSet, usize)>,
}

impl ArchetypeManager {
    pub fn new() -> Self {
        Self {
            archetypes: HashMap::new(),
            entity_locations: HashMap::new(),
        }
    }

    pub fn get_or_create_archetype(
        &mut self,
        component_set: &ComponentSet,
        component_registry: &ComponentRegistry,
    ) -> &mut Archetype {
        if !self.archetypes.contains_key(component_set) {
            let mut types = Vec::<ComponentInfo>::with_capacity(component_set.bits.len());
            for i in 0..component_set.bits.len() {
                // TODO: This smells very fishy
                if component_set.contains(i) {
                    let info = component_registry
                        .get_info(i)
                        .expect("Component not registered");
                    types.push(info.clone());
                }
            }
            self.archetypes
                .insert(component_set.clone(), Archetype::new(types, 64));
        }
        self.archetypes.get_mut(component_set).unwrap()
    }

    pub fn insert(
        &mut self,
        entity: Entity,
        component_set: &ComponentSet,
        components: &[(*const u8, ComponentId)],
        component_registry: &ComponentRegistry,
    ) {
        let len = {
            let archetype = self.get_or_create_archetype(component_set, component_registry);
            unsafe {
                archetype.insert(entity, components);
            }
            archetype.len()
        };

        self.entity_locations
            .insert(entity, (component_set.clone(), len - 1));
    }

    pub unsafe fn remove_entity(&mut self, entity: Entity) {
        if let Some((component_set, index)) = self.entity_locations.remove(&entity) {
            if let Some(archetype) = self.archetypes.get_mut(&component_set) {
                unsafe {
                    archetype.remove(index);
                }

                if index < archetype.entities.len() {
                    let moved_entity = archetype.entities[index];
                    self.entity_locations
                        .insert(moved_entity, (component_set.clone(), index));
                }
            }
        }
    }
}
