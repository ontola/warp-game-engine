//! Small deterministic PCG32: the simulation must replay bit-for-bit.

#[derive(Clone, Debug)]
pub struct Rng { state: u64 }

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut r = Self { state: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x853C_49E6_748F_EA9B };
        r.next_u32();
        r
    }
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }
    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f32 { (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32 }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 { lo + (hi - lo) * self.f() }
    pub fn below(&mut self, n: u32) -> u32 { if n == 0 { 0 } else { self.next_u32() % n } }
}
