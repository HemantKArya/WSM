//! GPU acceptance helper: drag a window title bar with the instance's USB tablet over QMP, the way a
//! user would (round 2 G1 method). Test harness only; the product never injects input.
//! usage: qmp_drag <instance dir> <x> <y> [cycles] [width height]
use anyhow::Result;
use serde_json::json;
use std::{path::PathBuf, time::{Duration, Instant}};
use wsm_core::qmp::Qmp;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(a.get(1).expect("instance dir"));
    let num = |i: usize, d: f64| a.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(d);
    let (x0, y0, cycles, w, h) = (num(2, 800.0), num(3, 210.0), num(4, 6.0) as u32, num(5, 1920.0), num(6, 1080.0));
    let mut q = Qmp::connect(&dir.join("qmp.sock"), Duration::from_secs(5))?;
    let mut send = |events: serde_json::Value| q.execute("input-send-event", json!({"events": events}));
    let abs = |x: f64, y: f64| json!([
        {"type": "abs", "data": {"axis": "x", "value": (x * 32767.0 / (w - 1.0)) as i64}},
        {"type": "abs", "data": {"axis": "y", "value": (y * 32767.0 / (h - 1.0)) as i64}}]);
    let btn = |down: bool| json!([{"type": "btn", "data": {"button": "left", "down": down}}]);
    let start = Instant::now();
    for _ in 0..cycles {
        send(abs(x0, y0))?;
        std::thread::sleep(Duration::from_millis(100));
        send(btn(true))?;
        for i in 0..80 {
            let phase = i as f64 / 79.0;
            let dx = 240.0 * if phase < 0.5 { phase * 2.0 } else { (1.0 - phase) * 2.0 };
            send(abs(x0 + dx, y0 + dx / 5.0))?;
            std::thread::sleep(Duration::from_millis(40));
        }
        send(btn(false))?;
    }
    println!("{{\"drag_s\": {:.2}, \"cycles\": {cycles}}}", start.elapsed().as_secs_f64());
    Ok(())
}
