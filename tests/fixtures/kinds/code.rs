// Fixture for the #8 Part A test: struct/enum/trait must be distinguished,
// not all collapsed to Struct (tree-sitter-rust's tags.scm tags all of
// struct/enum/union/type-alias as "class").

struct Widget {
    id: u32,
}

enum Shape {
    Circle,
    Square,
}

trait Drawable {
    fn draw(&self);
}
