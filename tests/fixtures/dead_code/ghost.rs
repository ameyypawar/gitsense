// Fixture for the find_dead_code integration test.
//
// `referenced` is called by `caller`, so it has a reference in the index.
// `truly_dead` is never called — it must appear in unreferenced_defs().
// `caller` itself has no callers, so it also appears as dead; the test does
// not assert on it (callers of callers are out of scope for this fixture).

fn referenced() {
    let _ = 42;
}

fn truly_dead() {
    let _ = 0;
}

fn caller() {
    referenced();
}
