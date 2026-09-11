use crate::ComponentId;
use crate::component::Component;
use crate::registry::Registry;

/// A set of component values spawned together.
///
/// Implemented for every [`Component`] and for tuples of up to eight
/// components, including `()` for an entity with none. Field order is
/// irrelevant: `(A, B)` and `(B, A)` spawn identical entities.
///
/// # Safety
///
/// Implementors guarantee: `ids` and `write` must agree — `write` calls
/// `put` exactly once per component, in the same order as `ids`, with a
/// pointer valid for reading that component's type, and must not use the
/// values afterwards (ownership transfers through `put`).
pub unsafe trait Bundle {
    /// Registers and returns this bundle's component ids, in field order.
    fn ids(reg: &mut Registry) -> Vec<ComponentId>;

    /// Feeds each component value to `put`, in `ids` order, and forgets it.
    ///
    /// # Safety
    ///
    /// `put` must move each value out (or otherwise take ownership of the
    /// pointed-to bytes) before it returns.
    unsafe fn write(self, put: &mut dyn FnMut(*const u8));
}

unsafe impl<T: Component> Bundle for T {
    fn ids(reg: &mut Registry) -> Vec<ComponentId> {
        vec![reg.register::<T>()]
    }

    unsafe fn write(self, put: &mut dyn FnMut(*const u8)) {
        put((&raw const self).cast());
        core::mem::forget(self);
    }
}

macro_rules! tuple_bundle {
    ($(($t:ident, $v:ident)),*) => {
        unsafe impl<$($t: Component),*> Bundle for ($($t,)*) {
            #[allow(unused_variables)]
            fn ids(reg: &mut Registry) -> Vec<ComponentId> {
                vec![$(reg.register::<$t>(),)*]
            }

            #[allow(unused_variables)]
            unsafe fn write(self, put: &mut dyn FnMut(*const u8)) {
                let ($($v,)*) = self;
                $(
                    put((&raw const $v).cast());
                    core::mem::forget($v);
                )*
            }
        }
    }
}

tuple_bundle!();
tuple_bundle!((T1, v1));
tuple_bundle!((T1, v1), (T2, v2));
tuple_bundle!((T1, v1), (T2, v2), (T3, v3));
tuple_bundle!((T1, v1), (T2, v2), (T3, v3), (T4, v4));
tuple_bundle!((T1, v1), (T2, v2), (T3, v3), (T4, v4), (T5, v5));
tuple_bundle!((T1, v1), (T2, v2), (T3, v3), (T4, v4), (T5, v5), (T6, v6));
tuple_bundle!((T1, v1), (T2, v2), (T3, v3), (T4, v4), (T5, v5), (T6, v6), (T7, v7));
tuple_bundle!((T1, v1), (T2, v2), (T3, v3), (T4, v4), (T5, v5), (T6, v6), (T7, v7), (T8, v8));