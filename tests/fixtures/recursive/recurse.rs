fn a() { b(); }
fn b() { a(); }
fn entry() { a(); }
