use crate::component::{ComponentId, ComponentRegistry};
use crate::entity::Entity;
use std::alloc::Layout;
use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchetypeId(pub usize);

pub struct Column {
    data: Vec<u8>,
    layout: Layout,
}

impl Column {
    pub fn new(layout: Layout) -> Self {
        Self {
            data: Vec::new(),
            layout,
        }
    }

    pub fn len(&self) -> usize {
        if self.layout.size() == 0 {
            return self.data.len();
        }
        self.data.len() / self.layout.size()
    }

    pub unsafe fn push(&mut self, component_ptr: *const u8) {
        let size = self.layout.size();
        if size > 0 {
            unsafe {
                let src_slice = std::slice::from_raw_parts(component_ptr, size);
                self.data.extend_from_slice(src_slice);
            }
        } else {
            // For zero-sized types, we just push a placeholder
            self.data.push(0);
        }
    }

    pub unsafe fn swap_remove(&mut self, row: usize) {
        let size = self.layout.size();
        let last_index = self.len() - 1;

        if size > 0 {
            if row != last_index {
                unsafe {
                    let row_ptr = self.data.as_mut_ptr().add(row * size);
                    let last_ptr = self.data.as_ptr().add(last_index * size);
                    std::ptr::copy_nonoverlapping(last_ptr, row_ptr, size);
                }
            }
            let new_len = self.data.len() - size;
            unsafe {
                self.data.set_len(new_len);
            }
        } else {
            // For zero-sized types, we just remove the last element
            self.data.pop();
        }
    }

    pub fn get_ptr(&self, row: usize) -> *const u8 {
        let size = self.layout.size();

        unsafe { self.data.as_ptr().add(row * size) }
    }

    pub fn get_mut_ptr(&self, row: usize) -> *mut u8 {
        self.get_ptr(row) as *mut u8
    }
}

pub struct Archetype {
    id: ArchetypeId,
    columns: HashMap<ComponentId, Column>,
    entities: Vec<Entity>,
}

impl Archetype {
    pub fn new(id: ArchetypeId) -> Self {
        Self {
            id,
            columns: HashMap::new(),
            entities: Vec::new(),
        }
    }

    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn columns(&self) -> &HashMap<ComponentId, Column> {
        &self.columns
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Adds a new entity to the archetype, along with its components.
    /// Returns the row index where the entity was inserted.
    ///
    /// # Safety
    /// The `component_data` pointers must be valid and must correspond to the `ComponentId`s
    pub unsafe fn add(
        &mut self,
        entity: Entity,
        component_data: &[(ComponentId, *const u8)],
        registry: &ComponentRegistry,
    ) -> usize {
        for (id, ptr) in component_data {
            let column = self.columns.entry(*id).or_insert_with(|| {
                let info = registry
                    .get_info(*id)
                    .expect("Component must be registered before being added to an archetype");
                Column::new(info.layout)
            });

            unsafe {
                column.push(*ptr);
            }
        }

        let row = self.len();
        self.entities.push(entity);
        row
    }

    /// Removes an entity from the specified row using `swap_remove`.
    ///
    /// # Returns
    /// A tuple containing:
    /// 1.  The `Entity` that was at the specified `row` (the one being removed).
    /// 2.  An `Option<Entity>` containing the `Entity` that was moved from the ned of the list
    ///     to replace the removed one. This is `None` if the removed entity was the last one.
    ///     The `World` needs this information to update the moved entity's `EntityLocation`.
    pub fn remove(&mut self, row: usize) -> (Entity, Option<Entity>) {
        for column in self.columns.values_mut() {
            unsafe {
                column.swap_remove(row);
            }
        }

        let removed_entity = self.entities.remove(row);

        let moved_entity = if row < self.entities.len() {
            Some(self.entities[row])
        } else {
            None
        };

        (removed_entity, moved_entity)
    }

    /// Adds an entity to this archetype by copying all of its existing component
    /// data from a source archetype.
    ///
    /// Returns the new row index of the added entity.
    ///
    /// # Safety
    /// The caller must uphold several invariants:
    /// 1. `source_row` must be a valid, in-bounds row index for the `source_archetype`.
    /// 2. For every `ComponentId` present in `self.columns`, if that component also
    ///    exists in the `source_archetype`, the `source_column` must be valid.
    /// 3. The `source_archetype` reference must be valid and distinct from `self`.
    pub unsafe fn add_moved_entity(
        &mut self,
        entity: Entity,
        source_archetype: &Archetype,
        source_row: usize,
    ) -> usize {
        let new_row = self.len();

        for (component_id, target_column) in &mut self.columns {
            if let Some(source_column) = source_archetype.columns.get(component_id) {
                let component_ptr = source_column.get_ptr(source_row);
                unsafe {
                    target_column.push(component_ptr);
                }
            }
        }

        self.entities.push(entity);

        debug_assert!(
            self.columns
                .values()
                .all(|column| column.len() == self.len())
        );

        new_row
    }

    pub fn has_component(&self, component_id: ComponentId) -> bool {
        // TODO: This is a linear search, consider optimizing with a HashSet or similar structure
        self.columns.contains_key(&component_id)
    }
}
