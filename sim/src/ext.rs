//! External bot controller: runs an opponent as a sandboxed process speaking mechfight/1 on stdin/stdout.
//! Opponents are untrusted code, so the normal launcher is `docker run` with the official match sandbox flags.
use mech_bot::protocol::{Action, Obs};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub struct ExtBot {
    pub name: String,
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    pub alive: bool,
    pub late: u64,
    pub answered: u64,
    pending_tick: Option<u64>,
}

/// Build the launch command for an opponent image (docker) under the official sandbox limits.
pub fn docker_cmd(image: &str, arena_host_dir: &str) -> Command {
    let mut c = Command::new("docker");
    c.args([
        "run", "--rm", "-i", "--network", "none", "--cpus", "1", "--memory", "512m", "--read-only", "--tmpfs",
        "/tmp:rw,size=64m", "--user", "65534:65534", "-v",
    ]);
    c.arg(format!("{arena_host_dir}:/arena:ro"));
    c.arg(image);
    c
}

impl ExtBot {
    pub fn spawn(name: &str, mut cmd: Command) -> Option<ExtBot> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().ok()?;
        let stdout = child.stdout.take()?;
        let stdin = child.stdin.take();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Some(ExtBot { name: name.to_string(), child, stdin, rx, alive: true, late: 0, answered: 0, pending_tick: None })
    }

    fn send(&mut self, v: &Value) {
        if let Some(w) = self.stdin.as_mut() {
            let mut s = serde_json::to_string(v).unwrap_or_default();
            s.push('\n');
            if w.write_all(s.as_bytes()).and_then(|_| w.flush()).is_err() {
                self.alive = false;
            }
        }
    }

    /// Send hello and wait for `ready` (container start included).
    pub fn hello(&mut self, slot: usize, seed: u64, config: &Value, budget: Duration) -> bool {
        let h = json!({"type":"hello","protocol":"mechfight/1","slot":slot,"seed":seed,
            "map":{"dir":"/arena","files":["v2/world.json","v2/terrain.i16"]},"config":config});
        self.send(&h);
        let t0 = Instant::now();
        while t0.elapsed() < budget {
            match self.rx.recv_timeout(budget.saturating_sub(t0.elapsed())) {
                Ok(l) => {
                    if l.contains("\"ready\"") {
                        return true;
                    }
                }
                Err(_) => break,
            }
        }
        self.alive = false;
        false
    }

    /// Lockstep phase 1: ship the observation.
    pub fn send_obs(&mut self, ob: &Obs) {
        if !self.alive {
            return;
        }
        let mut v = serde_json::to_value(ob).unwrap_or(Value::Null);
        if let Some(m) = v.as_object_mut() {
            m.insert("type".into(), json!("obs"));
        }
        self.pending_tick = Some(ob.tick);
        self.send(&v);
    }

    /// Lockstep phase 2: wait (until `deadline`) for the answer echoing the pending tick.
    pub fn recv_action(&mut self, deadline: Instant) -> Option<Action> {
        let tick = self.pending_tick.take()?;
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.late += 1;
                return None;
            }
            match self.rx.recv_timeout(deadline - now) {
                Ok(l) => {
                    let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
                    if v["tick"].as_f64().map(|t| t as u64) != Some(tick) {
                        continue; // stale answer to an older observation
                    }
                    self.answered += 1;
                    return Some(parse_action(&v, tick));
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.late += 1;
                    return None;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.alive = false;
                    return None;
                }
            }
        }
    }

    pub fn end(&mut self, place: u32, kills: u32) {
        self.send(&json!({"type":"end","place":place,"kills":kills}));
        self.stdin = None;
    }
}

impl Drop for ExtBot {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn v2(v: &Value) -> Option<[f64; 2]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

pub fn parse_action(v: &Value, tick: u64) -> Action {
    let fire = v["fire"].as_array().map(|a| [a.first().and_then(|x| x.as_bool()).unwrap_or(false), a.get(1).and_then(|x| x.as_bool()).unwrap_or(false)]);
    Action {
        tick,
        mv: v2(&v["move"]),
        boost: v["boost"].as_bool(),
        jump: v["jump"].as_bool(),
        aim: v2(&v["aim"]),
        fire,
        reload: v["reload"].as_bool(),
        pick: v["pick"].as_f64().map(|x| x as usize),
        take: v["take"].as_f64().map(|x| x as i64),
        drop: v["drop"].as_f64().map(|x| x as usize),
    }
}
