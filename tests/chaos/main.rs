//! The chaos suite: tests the `qa` machine writes against decree (.decree/lib/qa/qa.sh,
//! `QA_RUN="cargo test --test chaos"`). Each names its wrench and the invariant it checks
//! (.decree/lib/qa/wrenches.md, invariants.md). A real bug's test is marked
//! `#[should_panic]` until a `develop` run fixes it; the mark comes off with the fix.
//! Empty until `qa` first runs.
