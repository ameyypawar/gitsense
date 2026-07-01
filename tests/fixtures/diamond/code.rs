fn a() {
    b();
    c();
}

fn b() {
    c();
}

fn c() {}
