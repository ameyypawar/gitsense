struct S;
impl S {
    fn m(&self) {}
    fn make() -> S {
        S
    }
}
fn driver() {
    let s = S::make();
    s.m();
}
