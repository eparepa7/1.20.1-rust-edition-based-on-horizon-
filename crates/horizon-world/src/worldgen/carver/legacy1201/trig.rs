//! Mth.sin/cos from 1.20.1: float multiply/add, then Java float-to-int.
//! Kept separate from 26.3's double-indexed table and its callers.
use std::sync::OnceLock;

pub fn sine_table() -> &'static [f32] {
    static TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..65536)
            .map(|index| {
                horizon_core::jmath::sin(index as f64 * std::f64::consts::PI * 2. / 65536.) as f32
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

pub fn sin(angle: f32) -> f32 {
    sine_table()[((angle * 10430.378_f32) as i32 & 65535) as usize]
}
pub fn cos(angle: f32) -> f32 {
    sine_table()[((angle * 10430.378_f32 + 16384_f32) as i32 & 65535) as usize]
}
