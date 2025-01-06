use flux_engine_macros::memory_region;
use flux_engine_memory::{Region, GLOBAL};

#[test]
fn test_single_function() {
    let allocation_count = GLOBAL.get_count(Region::Graphics);

    #[memory_region(Region::Graphics)]
    fn test_fn() {
        let _vec = Vec::<i32>::new();
    }

    test_fn();
    assert_eq!(GLOBAL.get_count(Region::Graphics), allocation_count + 1);
}
