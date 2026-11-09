//! cargo run -q --release -p plonk-host --example shot -- <exe> <out.ppm> [mouse events "d,x,y;m,x,y;u,x,y"]
use plonk_host::wine::WineProc;
const APP: &str = r"c:pp.exe";
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut p = WineProc::new();
    p.load_dir("wine/out");
    p.files.insert(APP.into(), std::fs::read(&a[1]).unwrap());
    p.boot(APP).unwrap();
    println!("{:?}", p.run_until_idle(1_000_000));
    for ev in a.get(3).map(|s| s.as_str()).unwrap_or("").split(';').filter(|s| !s.is_empty()) {
        let f: Vec<&str> = ev.split(',').collect();
        let k = match f[0] { "d" => 0, "m" => 1, _ => 2 };
        p.mouse(k, f[1].parse().unwrap(), f[2].parse().unwrap());
        println!("{:?}", p.run_until_idle(1_000_000));
    }
    let px = p.compose_rgba(1024, 768);
    let mut out = b"P6\n1024 768\n255\n".to_vec();
    for c in px.chunks(4) { out.extend_from_slice(&c[..3]); }
    std::fs::write(&a[2], out).unwrap();
}
