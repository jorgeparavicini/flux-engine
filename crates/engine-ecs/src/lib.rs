use std::any::TypeId;
use std::hash::{Hash, Hasher};
use std::num::NonZero;

#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct Entity {
    #[cfg(target_endian = "little")]
    index: u32,
    generation: NonZero<u32>,
    #[cfg(target_endian = "big")]
    index: u32,
}

impl PartialEq for Entity {
    #[inline]
    fn eq(&self, other: &Entity) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl Eq for Entity {}

impl PartialOrd for Entity {
    #[inline]
    fn partial_cmp(&self, other: &Entity) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entity {
    #[inline]
    fn cmp(&self, other: &Entity) -> std::cmp::Ordering {
        self.to_bits().cmp(&other.to_bits())
    }
}

impl Hash for Entity {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_bits().hash(state)
    }
}

pub struct ComponentVec {
    data: Vec<u8>,
    component_size: usize,
    type_id: TypeId,
}

impl ComponentVec {
    fn new<T: 'static>() -> Self {
        Self {
            data: Vec::new(),
            component_size: size_of::<T>(),
            type_id: TypeId::of::<T>(),
        }
    }

    fn push<T: 'static>(&mut self, component: T) -> usize{
        assert_eq!(self.type_id, TypeId::of::<T>());
        let index = self.data.len() / self.component_size;

        unsafe {
            let ptr = std::ptr::from_ref::<T>(&component).cast::<u8>();
            self.data
                .extend_from_slice(std::slice::from_raw_parts(ptr, self.component_size));
        }
        std::mem::forget(component);

        index
    }
    
    unsafe fn 
}
