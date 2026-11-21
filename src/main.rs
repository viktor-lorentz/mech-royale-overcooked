//! Overcooked mech bot: mechfight/1 over stdin/stdout.
mod brain;
mod geom;
mod protocol;

use protocol::*;
use std::io::{BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Instant;

fn send(out: &mut impl Write, line: &str) {
    let _ = out.write_all(line.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

fn main() {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) if !l.trim().is_empty() => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut bot: Option<brain::Brain> = None;
    let mut last_cont = Action::default();
    let mut lat: Vec<f64> = Vec::with_capacity(5000);
    let mut panics = 0u32;

    'outer: loop {
        let first = match rx.recv() {
            Ok(l) => l,
            Err(_) => break,
        };
        let mut batch = vec![first];
        while let Ok(l) = rx.try_recv() {
            batch.push(l);
        }
        let started = Instant::now();
        let mut newest_obs: Option<String> = None;
        for line in batch {
            let tag: Tag = match serde_json::from_str(&line) {
                Ok(t) => t,
                Err(_) => continue,
            };
            match tag.t.as_str() {
                "hello" => {
                    if let Ok(h) = serde_json::from_str::<Hello>(&line) {
                        bot = catch_unwind(AssertUnwindSafe(|| brain::Brain::new(&h))).ok();
                    }
                    send(&mut out, r#"{"type":"ready","name":"overcooked"}"#);
                }
                "obs" => newest_obs = Some(line),
                "end" => {
                    if let Ok(e) = serde_json::from_str::<End>(&line) {
                        lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        let n = lat.len().max(1);
                        eprintln!(
                            "END place={} kills={} decisions={} lat_ms p50={:.2} p99={:.2} max={:.2} panics={}",
                            e.place,
                            e.kills,
                            lat.len(),
                            lat.get(n / 2).copied().unwrap_or(0.0),
                            lat.get(n * 99 / 100).copied().unwrap_or(0.0),
                            lat.last().copied().unwrap_or(0.0),
                            panics
                        );
                    }
                    break 'outer;
                }
                _ => {}
            }
        }
        let Some(line) = newest_obs else { continue };
        let obs: Obs = match serde_json::from_str(&line) {
            Ok(o) => o,
            Err(_) => continue,
        };
        let action = match bot.as_mut() {
            Some(b) => match catch_unwind(AssertUnwindSafe(|| b.act(&obs))) {
                Ok(a) => {
                    last_cont = Action { tick: 0, mv: a.mv, boost: a.boost, aim: a.aim, ..Default::default() };
                    a
                }
                Err(_) => {
                    panics += 1;
                    Action { tick: obs.tick, ..last_cont.clone() }
                }
            },
            None => Action { tick: obs.tick, ..last_cont.clone() },
        };
        match serde_json::to_string(&action) {
            Ok(s) => send(&mut out, &s),
            Err(_) => send(&mut out, &format!(r#"{{"tick":{}}}"#, obs.tick)),
        }
        lat.push(started.elapsed().as_secs_f64() * 1000.0);
    }
}
