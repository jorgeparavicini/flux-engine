use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Region {
    Graphics,
    Physics,
    Audio,
    Scene,
    General,
}

thread_local! {
    static CURRENT_REGION: RefCell<Region> = const { RefCell::new(Region::General) };
}

pub struct RegionGuard {
    previous: Region,
}

impl RegionGuard {
    #[must_use]
    pub fn new(region: Region) -> Self {
        let previous_region = CURRENT_REGION.with(|current| {
            let prev = *current.borrow();
            *current.borrow_mut() = region;
            prev
        });

        Self {
            previous: previous_region,
        }
    }
}

impl Drop for RegionGuard {
    fn drop(&mut self) {
        CURRENT_REGION.with(|current| {
            *current.borrow_mut() = self.previous;
        });
    }
}

pub fn get_current_region() -> Region {
    CURRENT_REGION.with(|r| *r.borrow())
}
