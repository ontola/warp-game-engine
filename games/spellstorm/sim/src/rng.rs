//! The game's random numbers: Godot's PCG (gdcompat), with the calls the
//! GDScript makes spelled short.

pub use gdcompat::Pcg;

pub trait Rand {
    /// `randf()`, 0 to 1.
    fn f(&mut self) -> f32;
    /// `randf_range(a, b)`.
    fn range(&mut self, a: f32, b: f32) -> f32;
    /// `randi() % n`.
    fn pick(&mut self, n: usize) -> usize;
    /// `randi_range(a, b)`, both ends included.
    fn int(&mut self, a: i32, b: i32) -> i32;
    /// A coin with this chance of heads.
    fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    /// +1 or -1, evenly.
    fn sign(&mut self) -> f32 {
        if self.f() < 0.5 { 1.0 } else { -1.0 }
    }
}

impl Rand for Pcg {
    fn f(&mut self) -> f32 {
        self.randf()
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        self.randf_range_f32(a, b)
    }
    fn pick(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            self.next_u32() as usize % n
        }
    }
    fn int(&mut self, a: i32, b: i32) -> i32 {
        self.randi_range(a, b)
    }
}
