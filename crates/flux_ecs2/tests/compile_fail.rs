//! Things the derive must REJECT at compile time. First run:
//!   TRYBUILD=overwrite cargo test -p flux_ecs2 --test compile_fail
//! then review every generated tests/compile_fail/*.stderr and commit them.
#[test]
#[cfg_attr(miri, ignore = "trybuild spawns rustc, which miri cannot interpret")]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
