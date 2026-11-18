//! Live progress of machine setup (`sh::ensure`), read by the setup page.
//!
//! The sandbox image build is the long part. Its steps come from the embedded
//! Dockerfile, weighted by how long they usually take, and BuildKit's plain
//! output says which step runs and how many bytes the base images have left.
//! The page gets `lo` (work proven done) and `hi` (where the current step ends)
//! as percentages and trickles between them, so the bar never jumps back or
//! runs past what is known.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

static ALL: LazyLock<Mutex<HashMap<String, Value>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Percent bands: checks, image build, container start.
const CHECK: (f64, f64) = (0.0, 3.0);
const BUILD: (f64, f64) = (3.0, 92.0);
const START: (f64, f64) = (92.0, 99.0);

pub fn get(id: &str) -> Value {
    ALL.lock().unwrap().get(id).cloned().unwrap_or_else(|| json!({"phase": "idle"}))
}

fn put(id: &str, v: Value) {
    ALL.lock().unwrap().insert(id.to_string(), v);
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// One step of the final Dockerfile stage, as BuildKit numbers them.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub label: &'static str,
    pub w: f64,
}

/// The final stage's FROM, RUN, COPY, ADD and WORKDIR instructions, in order.
/// ENV, ARG and the like get no step number in BuildKit, so they are skipped.
pub fn steps(df: &str) -> Vec<Step> {
    let mut ins: Vec<String> = Vec::new();
    let mut cur = String::new();
    for l in df.lines() {
        let t = l.trim();
        if cur.is_empty() && (t.is_empty() || t.starts_with('#')) {
            continue;
        }
        if let Some(x) = t.strip_suffix('\\') {
            cur.push_str(x);
            cur.push(' ');
        } else {
            cur.push_str(t);
            ins.push(std::mem::take(&mut cur));
        }
    }
    let last = ins.iter().rposition(|i| i.to_uppercase().starts_with("FROM ")).unwrap_or(0);
    ins[last..]
        .iter()
        .filter_map(|i| {
            let (op, rest) = i.split_once(' ').unwrap_or((i, ""));
            let r = rest.to_lowercase();
            Some(match op.to_uppercase().as_str() {
                "FROM" => Step { label: "Downloading base images", w: 30.0 },
                "RUN" if r.contains("apt-get") => Step { label: "Installing system packages", w: 34.0 },
                "RUN" if r.contains("rustup") || r.contains("npm install") => Step { label: "Adding Rust tools, pnpm and Playwright", w: 15.0 },
                "RUN" if r.contains("fnm") => Step { label: "Installing the Node version manager", w: 2.0 },
                "RUN" if r.contains("npm") => Step { label: "Setting up npm", w: 1.0 },
                "RUN" => Step { label: "Running a build step", w: 2.0 },
                "COPY" | "ADD" => Step { label: "Adding Node.js", w: 0.5 },
                "WORKDIR" => Step { label: "Finishing the image", w: 0.2 },
                _ => return None,
            })
        })
        .collect()
}

fn bytes(n: &str, u: &str) -> f64 {
    let n: f64 = n.parse().unwrap_or(0.0);
    n * match u.trim_end_matches('B').trim_end_matches('i') {
        "k" | "K" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        _ => 1.0,
    }
}

/// Follows one setup run. Feed it every output line; it publishes when the view changes.
pub struct Run {
    id: String,
    t0: i64,
    phase: &'static str,
    steps: Vec<Step>,
    /// BuildKit vertex `#N` → index into `steps`.
    vtx: HashMap<u32, usize>,
    pulls: HashSet<u32>,
    done: HashSet<usize>,
    /// (pull vertex, layer digest) → (bytes done, bytes total).
    layers: HashMap<(u32, String), (f64, f64)>,
    /// Legacy builder: `Step k/N`, equal weights.
    legacy: Option<(usize, usize)>,
    detail: String,
}

impl Run {
    pub fn new(id: &str, df: &str) -> Run {
        let r = Run { id: id.into(), t0: now(), phase: "check", steps: steps(df), vtx: HashMap::new(), pulls: HashSet::new(), done: HashSet::new(), layers: HashMap::new(), legacy: None, detail: String::new() };
        r.publish();
        r
    }

    fn cur(&self) -> usize {
        (0..self.steps.len()).find(|i| !self.done.contains(i)).unwrap_or(self.steps.len())
    }

    fn pulled(&self) -> (f64, f64) {
        self.layers.values().fold((0.0, 0.0), |(d, t), (a, b)| (d + a, t + b))
    }

    /// (lo, hi) inside the build band, 0..=1.
    fn build_frac(&self) -> (f64, f64) {
        if let Some((k, n)) = self.legacy {
            let n = n.max(1) as f64;
            return ((k as f64 - 1.0).max(0.0) / n, (k as f64 / n).min(1.0));
        }
        let tw: f64 = self.steps.iter().map(|s| s.w).sum::<f64>().max(1e-9);
        let dw: f64 = self.done.iter().filter_map(|i| self.steps.get(*i)).map(|s| s.w).sum();
        let c = self.cur();
        let Some(s) = self.steps.get(c) else { return (1.0, 1.0) };
        // Downloads are most of a FROM step; unpacking the layers is the rest.
        let part = if s.label.starts_with("Downloading") {
            let (d, t) = self.pulled();
            if t > 0.0 { 0.85 * (d / t).min(1.0) } else { 0.0 }
        } else {
            0.0
        };
        ((dw + s.w * part) / tw, (dw + s.w) / tw)
    }

    pub fn view(&self) -> Value {
        let el = now() - self.t0;
        let band = |b: (f64, f64), f: (f64, f64)| (b.0 + (b.1 - b.0) * f.0, b.0 + (b.1 - b.0) * f.1);
        let (lo, hi, label, k, n) = match self.phase {
            "check" => (CHECK.0, CHECK.1, "Checking Docker".to_string(), 0, 0),
            "build" => {
                let (lo, hi) = band(BUILD, self.build_frac());
                let (k, n, label) = match self.legacy {
                    Some((k, n)) => (k, n, format!("Building the sandbox image ({k} of {n})")),
                    None => {
                        let c = self.cur().min(self.steps.len().saturating_sub(1));
                        let mut l = self.steps.get(c).map(|s| s.label.to_string()).unwrap_or_else(|| "Building the sandbox image".into());
                        let (d, t) = self.pulled();
                        if l.starts_with("Downloading") && t > 0.0 {
                            l = format!("{l} · {} of {}", size(d), size(t));
                        }
                        (c + 1, self.steps.len(), l)
                    }
                };
                (lo, hi, label, k, n)
            }
            "start" => (START.0, START.1, "Starting the sandbox".to_string(), 0, 0),
            _ => (100.0, 100.0, String::new(), 0, 0),
        };
        json!({"phase": self.phase, "label": label, "detail": self.detail, "k": k, "n": n, "lo": (lo * 10.0).round() / 10.0, "hi": (hi * 10.0).round() / 10.0, "t0": self.t0, "el": el})
    }

    fn publish(&self) {
        put(&self.id, self.view());
    }

    pub fn line(&mut self, l: &str) {
        let before = (self.phase, self.cur(), self.done.len(), self.legacy, (self.pulled().0 / 5e6) as i64, self.detail.clone());
        self.parse(l.trim());
        let after = (self.phase, self.cur(), self.done.len(), self.legacy, (self.pulled().0 / 5e6) as i64, self.detail.clone());
        if before != after {
            self.publish();
        }
    }

    fn parse(&mut self, l: &str) {
        match l {
            "@@build" => return self.phase = "build",
            "@@start" => return self.phase = "start",
            _ => {}
        }
        // Legacy builder: "Step 3/12 : RUN ...".
        if let Some(r) = l.strip_prefix("Step ") {
            if let Some((kn, _)) = r.split_once(" :") {
                if let Some((k, n)) = kn.split_once('/') {
                    if let (Ok(k), Ok(n)) = (k.parse(), n.parse()) {
                        self.phase = "build";
                        self.legacy = Some((k, n));
                    }
                }
            }
            return;
        }
        let Some(r) = l.strip_prefix('#') else { return };
        let Some((v, rest)) = r.split_once(' ') else { return };
        let Ok(v) = v.parse::<u32>() else { return };
        self.phase = "build";
        if let Some(r) = rest.strip_prefix('[') {
            // "[stage-1 3/10] RUN ..." or "[node24 1/1] FROM ...".
            let Some((head, ins)) = r.split_once("] ") else { return };
            if ins.starts_with("FROM ") {
                self.pulls.insert(v);
            }
            if let Some((stage, kn)) = head.rsplit_once(' ') {
                if let Some((k, n)) = kn.split_once('/') {
                    if let (Ok(k), Ok(n)) = (k.parse::<usize>(), n.parse::<usize>()) {
                        if stage.starts_with("stage-") && n == self.steps.len() && k >= 1 {
                            self.vtx.insert(v, k - 1);
                            self.detail.clear();
                        }
                    }
                }
            }
            return;
        }
        if rest == "DONE" || rest.starts_with("DONE ") || rest == "CACHED" {
            if let Some(&i) = self.vtx.get(&v) {
                self.done.insert(i);
                self.detail.clear();
            }
            if self.pulls.contains(&v) {
                for (k, x) in self.layers.iter_mut() {
                    if k.0 == v {
                        x.0 = x.1;
                    }
                }
            }
            return;
        }
        if self.pulls.contains(&v) {
            // "sha256:ab12… 52.43MB / 211.32MB 3.2s"
            let w: Vec<&str> = rest.split_whitespace().collect();
            if w.len() >= 4 && w[0].starts_with("sha256:") && w[2] == "/" {
                let num = |s: &str| {
                    let i = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
                    bytes(&s[..i], &s[i..])
                };
                self.layers.insert((v, w[0].to_string()), (num(w[1]), num(w[3])));
            } else if w.first() == Some(&"extracting") {
                self.detail = "Unpacking".into();
            }
            return;
        }
        // RUN output: "#6 12.34 Setting up clang-19 (1:19.1.7-3) ...". Show what it is doing right now.
        if self.vtx.get(&v) == Some(&self.cur()) {
            if let Some((ts, out)) = rest.split_once(' ') {
                if ts.parse::<f64>().is_ok() {
                    let o = out.trim();
                    let o = o.strip_prefix("Setting up ").or_else(|| o.strip_prefix("Unpacking ")).map(|p| p.split(' ').next().unwrap_or(p)).map(|p| format!("Installing {p}")).unwrap_or_else(|| o.to_string());
                    if !o.is_empty() {
                        self.detail = o.chars().take(80).collect();
                    }
                }
            }
        }
    }

    pub fn finish(mut self, ok: Result<(), String>) {
        let el = now() - self.t0;
        self.phase = if ok.is_ok() { "done" } else { "error" };
        let mut v = self.view();
        v["el"] = json!(el);
        if let Err(m) = ok {
            v["m"] = json!(m);
        }
        put(&self.id, v);
    }
}

fn size(b: f64) -> String {
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else {
        format!("{:.0} MB", b / 1e6)
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn steps_follow_the_final_stage_like_buildkit() {
        let s = steps(crate::sh::DF);
        let l: Vec<&str> = s.iter().map(|x| x.label).collect();
        assert_eq!(l.len(), 10, "{l:?}");
        assert_eq!(l[0], "Downloading base images");
        assert_eq!(l[1], "Installing system packages");
        assert_eq!(l[9], "Finishing the image");
    }

    #[test]
    fn a_build_moves_forward_through_downloads_and_steps() {
        let id = "t-prog";
        let mut r = Run::new(id, crate::sh::DF);
        assert_eq!(get(id)["phase"], "check");
        let mut last = 0.0;
        let mut seen = vec![];
        for l in [
            "@@build",
            "#1 [internal] load build definition from Dockerfile",
            "#4 [node24 1/1] FROM docker.io/library/node:24-trixie-slim@sha256:aa",
            "#5 [stage-1 1/10] FROM docker.io/library/rust:1-trixie@sha256:bb",
            "#5 sha256:1111111111111111 0B / 500.00MB 0.2s",
            "#4 sha256:2222222222222222 10.00MB / 100.00MB 0.4s",
            "#5 sha256:1111111111111111 250.00MB / 500.00MB 9.1s",
            "#5 extracting sha256:1111111111111111",
            "#5 DONE 40.1s",
            "#4 DONE 12.0s",
            "#6 [stage-1 2/10] RUN apt-get update && apt-get install -y",
            "#6 31.2 Setting up clang-19 (1:19.1.7-3) ...",
            "#6 DONE 120.0s",
            "#7 [stage-1 3/10] COPY --from=node24 /usr/local/bin/node /usr/local/bin/node",
            "#7 CACHED",
            "@@start",
        ] {
            r.line(l);
            let v = get(id);
            let lo = v["lo"].as_f64().unwrap();
            assert!(lo >= last, "{l}: {lo} < {last}");
            assert!(v["hi"].as_f64().unwrap() >= lo);
            last = lo;
            seen.push((v["label"].as_str().unwrap().to_string(), v["detail"].as_str().unwrap().to_string()));
        }
        assert!(seen.iter().any(|(l, _)| l == "Downloading base images · 260 MB of 600 MB"), "{seen:?}");
        assert!(seen.iter().any(|(l, d)| l == "Installing system packages" && d == "Installing clang-19"), "{seen:?}");
        assert_eq!(get(id)["label"], "Starting the sandbox");
        r.finish(Ok(()));
        assert_eq!((get(id)["phase"].as_str(), get(id)["lo"].as_f64()), (Some("done"), Some(100.0)));
    }

    #[test]
    fn the_legacy_builder_counts_its_own_steps() {
        let mut r = Run::new("t-legacy", crate::sh::DF);
        r.line("Step 3/12 : RUN apt-get update");
        let v = get("t-legacy");
        assert_eq!((v["k"].as_u64(), v["n"].as_u64()), (Some(3), Some(12)));
        assert!(v["lo"].as_f64().unwrap() < v["hi"].as_f64().unwrap());
        r.finish(Err("boom".into()));
        assert_eq!(get("t-legacy")["m"], "boom");
    }
}
