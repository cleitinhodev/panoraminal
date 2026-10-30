//! Prints what Panoraminal imports from brush files:
//! `cargo run -p panoraminal-io --example brushinfo -- file.abr [more files…]`

fn main() {
    for a in std::env::args().skip(1) {
        let p = std::path::Path::new(&a);
        match panoraminal_io::brushes::import_brush_file(p) {
            Ok(v) => {
                println!("{a}: {} brush(es)", v.len());
                for b in v {
                    let tips: Vec<String> = b
                        .tips
                        .iter()
                        .map(|t| format!("{}x{}", t.width, t.height))
                        .collect();
                    println!(
                        "  {:?} dia={} tips=[{}] spacing={:.2} angle={} round={:.2} scatter={:.2} count={} sizeJ={:.2} press_size={} notices={:?}",
                        b.name,
                        b.diameter,
                        tips.join(","),
                        b.dynamics.spacing,
                        b.dynamics.angle,
                        b.dynamics.roundness,
                        b.dynamics.scatter,
                        b.dynamics.count,
                        b.dynamics.size_jitter,
                        b.pressure_size,
                        b.notices
                    );
                }
            }
            Err(e) => println!("{a}: error: {e}"),
        }
    }
}
