use plonk_display::*;
use std::time::Instant;
fn main() {
    let mut b = SoftBackend::new();
    let id = b.create_surface(648, 485);
    b.place(id, 50, 40, true);
    b.set_glass(id, 10, 24, 28);
    let t = Instant::now();
    for _ in 0..10 { std::hint::black_box(b.compose_desktop(1024, 768)); }
    println!("compose {:?}/frame", t.elapsed() / 10);
}
