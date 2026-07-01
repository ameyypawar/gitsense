// Fixture for the #8 Part B test: two methods named `new` in different
// impl blocks must not be silently conflated by name-based resolution.

struct Alpha {
    n: u32,
}

impl Alpha {
    fn new() -> Alpha {
        Alpha { n: 1 }
    }
}

struct Beta {
    n: u32,
}

impl Beta {
    fn new() -> Beta {
        Beta { n: 2 }
    }
}
