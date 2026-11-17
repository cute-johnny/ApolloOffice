use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn sh(d: &Path, c: &str) {
    let b = if cfg!(windows) { r"C:\Program Files\Git\bin\bash.exe" } else { "bash" };
    let o = Command::new(b).arg("-c").arg(c).current_dir(d).output().unwrap();
    assert!(o.status.success(), "{c}: {}", String::from_utf8_lossy(&o.stderr));
}

#[cfg(windows)]
fn wrap(d: &Path, n: &str, py: &Path) -> PathBuf {
    let f = d.join(format!("{n}.cmd"));
    std::fs::write(&f, format!("@python \"{}\" %*\r\n", py.display())).unwrap();
    f
}

#[cfg(unix)]
fn wrap(d: &Path, n: &str, py: &Path) -> PathBuf {
    let f = d.join(n);
    std::fs::write(&f, format!("#!/bin/sh\nexec python3 \"{}\" \"$@\"\n", py.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    f
}

fn furl(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}/")
    } else {
        format!("file:///{s}/")
    }
}

struct K(Child);
impl Drop for K {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn public_repo() {
    full_run(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn private_repo() {
    full_run(true).await;
}

async fn full_run(pv: bool) {
    let t = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("e2e-{}-{pv}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let rm = t.join("remotes");
    let fk = t.join("ghd");
    std::fs::create_dir_all(rm.join("o")).unwrap();
    std::fs::create_dir_all(&fk).unwrap();
    let seed = t.join("seed");
    std::fs::create_dir_all(seed.join("t")).unwrap();
    std::fs::write(seed.join("add.sh"), "add() { echo $(( $1 - $2 )); }\n").unwrap();
    std::fs::write(seed.join("t/run.sh"), "e=0\nfor f in t/*_test.sh; do bash \"$f\" || e=1; done\nexit $e\n").unwrap();
    std::fs::write(seed.join("t/zero_test.sh"), "source ./add.sh\n[ \"$(add 0 0)\" = 0 ] && echo 'ok 1 - zero' || { echo 'not ok 1 - zero'; exit 1; }\n").unwrap();
    sh(&seed, "git init -q -b main . && git -c user.name=t -c user.email=t@t add -A && git -c user.name=t -c user.email=t@t commit -q -m init");
    sh(&rm.join("o"), &format!("git clone -q --bare \"{}\" r.git", seed.display().to_string().replace('\\', "/")));
    std::fs::write(fk.join("issues.json"), json!([{"number": 1, "title": "add subtracts instead of adding", "body": "`add 2 3` prints -1, expected 5.", "author": {"login": "someone"}, "updatedAt": "2026-01-01T00:00:00Z", "labels": [], "assignees": [], "url": "https://github.com/o/r/issues/1"}]).to_string()).unwrap();
    let td = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let gw = wrap(&t, "gh", &td.join("fake-gh.py"));
    let cw = wrap(&t, "claude", &td.join("fake-claude.py"));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let data = t.join("data");
    let exe = env!("CARGO_BIN_EXE_apollo-office");
    let envs = [("APOLLO_OFFICE_GH", gw.display().to_string()), ("APOLLO_OFFICE_CLAUDE", cw.display().to_string()), ("APOLLO_OFFICE_REMOTE", furl(&rm)), ("APOLLO_OFFICE_FAST", "1".into()), ("FAKE_GH_DIR", fk.display().to_string()), ("FAKE_GH_PRIVATE", if pv { "1".into() } else { "0".into() })];
    let _k = K(Command::new(exe).args(["--data", &data.display().to_string(), "--port", &port.to_string(), "--quiet"]).envs(envs.iter().cloned()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let l = Command::new(exe).args(["--data", &data.display().to_string(), "login-link"]).output().unwrap();
    let l = String::from_utf8_lossy(&l.stdout).lines().find(|x| x.starts_with("http")).unwrap().trim().to_string();
    let o = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = c.post(&l).header("origin", &o).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303);
    let ck = r.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    let get = |p: &str| {
        let c = c.clone();
        let u = format!("{o}{p}");
        let ck = ck.clone();
        async move { c.get(u).header("cookie", ck).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let post = |p: &str, b: Value| {
        let c = c.clone();
        let u = format!("{o}{p}");
        let ck = ck.clone();
        let o = o.clone();
        async move { c.post(u).header("cookie", ck).header("origin", o).header("x-ao", "1").json(&b).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    assert_eq!(c.get(format!("{o}/api/state")).send().await.unwrap().status().as_u16(), 401);
    assert_eq!(c.post(format!("{o}/api/act")).header("cookie", &ck).header("origin", "http://evil.example").header("x-ao", "1").json(&json!({"a": "pause", "on": true})).send().await.unwrap().status().as_u16(), 403);
    let h = post("/api/host", json!({"op": "set", "id": "local", "dk": false})).await;
    assert!(h["error"].is_string(), "direct mode must need confirmation");
    let h = post("/api/host", json!({"op": "set", "id": "local", "dk": false, "raw": true, "ln": 1})).await;
    assert_eq!(h["ok"], true, "{h}");
    let a = post("/api/repo", json!({"op": "add", "full": "o/r"})).await;
    assert_eq!(a["full"], "o/r", "{a}");
    post("/api/repo", json!({"op": "set", "full": "o/r", "ci": "bash t/run.sh", "hunt": false})).await;
    let t0 = Instant::now();
    let jid = loop {
        let s = get("/api/state").await;
        if let Some(j) = s["q"].as_array().unwrap().iter().find(|j| j["st"] == "approve") {
            break j["id"].as_i64().unwrap();
        }
        if let Some(j) = s["dn"].as_array().unwrap().iter().find(|j| j["st"] == "drop") {
            panic!("dropped: {j}\n{}", s["ev"]);
        }
        assert!(t0.elapsed() < Duration::from_secs(240), "no approval in time: {}", s["ev"]);
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let j = get(&format!("/api/job?id={jid}")).await;
    let pb = j["pb"].as_str().unwrap();
    assert!(pb.starts_with("Fixes #1"), "{pb}");
    assert!(pb.contains("`adds two numbers`: fails on `main` without the fix"), "{pb}");
    assert!(pb.contains("`bash t/run.sh` passes."), "{pb}");
    assert!(pb.contains("Generated with AI by [ApolloOffice Improved](https://github.com/cute-johnny/ApolloOfficeImproved)"), "{pb}");
    assert!(j["df"].as_str().unwrap().contains("+add() { echo $(( $1 + $2 )); }"), "{}", j["df"]);
    let r = post("/api/act", json!({"a": "approve", "id": jid, "pt": "Make add sum its arguments"})).await;
    assert!(r["error"].is_null(), "{r}");
    let t0 = Instant::now();
    loop {
        let s = get("/api/state").await;
        if let Some(x) = s["dn"].as_array().unwrap().iter().find(|x| x["id"] == jid) {
            assert_eq!(x["st"], "done", "{x}\n{}", s["ev"]);
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(120), "not submitted: {}", s["ev"]);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let ps: Value = serde_json::from_str(&std::fs::read_to_string(fk.join("prs.json")).unwrap()).unwrap();
    assert_eq!(ps.as_array().unwrap().len(), 1);
    assert_eq!(ps[0]["base"], "main");
    assert_eq!(ps[0]["title"], "Make add sum its arguments");
    let br = ps[0]["head"].as_str().unwrap().to_string();
    assert!(br.starts_with("fix/1-"), "{br}");
    let o = Command::new("git").args(["--git-dir", &rm.join("o/r.git").display().to_string(), "log", "-1", "--format=%B", &br]).output().unwrap();
    let m = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(m.starts_with("Make add sum its arguments\n\nFixes #1"), "{m}");
    assert!(m.contains("Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"), "{m}");
    assert!(m.contains("Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"), "{m}");
    let o = Command::new("git").args(["--git-dir", &rm.join("o/r.git").display().to_string(), "show", &format!("{br}:add.sh")]).output().unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("$1 + $2"));
    let s = get("/api/state").await;
    assert_eq!(s["pr"][0]["num"], 100);
}

/// The owner's chat messages become jobs at the right worker. The office is
/// paused so the jobs stay where the front desk put them.
#[tokio::test(flavor = "multi_thread")]
async fn front_desk_chat_routes_requests_to_workers() {
    let t = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("e2e-{}-desk", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let fk = t.join("ghd");
    std::fs::create_dir_all(&fk).unwrap();
    std::fs::write(fk.join("issues.json"), "[]").unwrap();
    let td = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let gw = wrap(&t, "gh", &td.join("fake-gh.py"));
    let cw = wrap(&t, "claude", &td.join("fake-claude.py"));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let data = t.join("data");
    let exe = env!("CARGO_BIN_EXE_apollo-office");
    let envs = [("APOLLO_OFFICE_GH", gw.display().to_string()), ("APOLLO_OFFICE_CLAUDE", cw.display().to_string()), ("APOLLO_OFFICE_REMOTE", furl(&t.join("remotes"))), ("APOLLO_OFFICE_FAST", "1".into()), ("FAKE_GH_DIR", fk.display().to_string())];
    let _k = K(Command::new(exe).args(["--data", &data.display().to_string(), "--port", &port.to_string(), "--quiet"]).envs(envs.iter().cloned()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let l = Command::new(exe).args(["--data", &data.display().to_string(), "login-link"]).output().unwrap();
    let l = String::from_utf8_lossy(&l.stdout).lines().find(|x| x.starts_with("http")).unwrap().trim().to_string();
    let o = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = c.post(&l).header("origin", &o).send().await.unwrap();
    let ck = r.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    let post = |p: &str, b: Value| {
        let (c, u, ck, o) = (c.clone(), format!("{o}{p}"), ck.clone(), o.clone());
        async move { c.post(u).header("cookie", ck).header("origin", o).header("x-ao", "1").json(&b).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    assert_eq!(c.post(format!("{o}/api/chat")).header("origin", &o).header("x-ao", "1").json(&json!({"msgs": []})).send().await.unwrap().status().as_u16(), 401);
    post("/api/act", json!({"a": "pause", "on": true})).await;
    let a = post("/api/repo", json!({"op": "add", "full": "o/r"})).await;
    assert_eq!(a["full"], "o/r", "{a}");
    // The front desk is off by default: the chat refuses until the owner turns it on.
    let off = c.post(format!("{o}/api/chat")).header("cookie", &ck).header("origin", &o).header("x-ao", "1").json(&json!({"msgs": [{"r": "you", "t": "hi"}]})).send().await.unwrap();
    assert_eq!(off.status().as_u16(), 409);
    assert_eq!(post("/api/set", json!({"orch": true})).await["set"]["orch"], true);

    let mut msgs = vec![];
    let mut say = |t: &str| {
        msgs.push(json!({"r": "you", "t": t}));
        json!({"msgs": msgs})
    };
    // A chat turn only proposes a plan; nothing starts before /api/chat/go.
    let mut acts = vec![];
    let r = post("/api/chat", say("Calling add 2 3 prints -1 instead of 5")).await;
    assert_eq!(r["reply"], "Repro will reproduce the bug.", "{r}");
    assert_eq!((r["plan"][0]["to"].as_str(), r["jobs"].as_array().map(|x| x.len())), (Some("repro"), Some(0)), "{r}");
    acts.push(r["plan"][0]["act"].clone());
    let r = post("/api/chat", say("Add a mul function")).await;
    assert_eq!((r["plan"][0]["to"].as_str(), r["plan"][0]["kind"].as_str()), (Some("repro"), Some("feature")), "{r}");
    acts.push(r["plan"][0]["act"].clone());
    let r = post("/api/chat", say("Please fix issue #1")).await;
    assert_eq!(r["plan"][0]["to"], "vet", "{r}");
    acts.push(r["plan"][0]["act"].clone());
    let r = post("/api/chat", say("Go hunt for bugs")).await;
    assert_eq!(r["plan"][0]["to"], "hunter", "{r}");
    acts.push(r["plan"][0]["act"].clone());
    let r = post("/api/chat", say("What about the other one?")).await;
    assert_eq!((r["reply"].as_str(), r["plan"].as_array().map(|x| x.len())), (Some("Which repository do you mean?"), Some(0)), "{r}");
    let r = post("/api/chat", say("fix the crash elsewhere")).await;
    assert_eq!(r["plan"].as_array().unwrap().len(), 0, "{r}");
    assert!(r["errs"][0].as_str().unwrap().contains("o/elsewhere is not added"), "{r}");
    let r = post("/api/chat", json!({"msgs": [{"r": "desk", "t": "hello"}]})).await;
    assert_eq!(r["error"], "write a message first", "{r}");

    let q = || async { c.get(format!("{o}/api/state")).header("cookie", &ck).send().await.unwrap().json::<Value>().await.unwrap()["q"].as_array().unwrap().len() };
    assert_eq!(q().await, 0, "the chat started work before the owner pressed Start");
    let keep: Vec<Value> = acts.iter().map(|a| json!({"act": a})).collect();
    let g = post("/api/chat/go", json!({"acts": keep})).await;
    assert_eq!(g["jobs"].as_array().unwrap().iter().filter(|j| j["new"] == true).count(), 4, "{g}");
    let g = post("/api/chat/go", json!({"acts": keep})).await;
    assert!(g["jobs"].as_array().unwrap().iter().all(|j| j["new"] == false), "{g}");

    let s = c.get(format!("{o}/api/state")).header("cookie", &ck).send().await.unwrap().json::<Value>().await.unwrap();
    let st: Vec<(String, String)> = s["q"].as_array().unwrap().iter().map(|j| (j["kind"].as_str().unwrap().to_string(), j["st"].as_str().unwrap().to_string())).collect();
    assert_eq!(st.len(), 4, "{}", s["q"]);
    for want in [("fix", "ready"), ("fix", "vet"), ("hunt", "ready")] {
        assert!(st.contains(&(want.0.to_string(), want.1.to_string())), "{want:?} missing in {}", s["q"]);
    }
    assert!(s["q"].as_array().unwrap().iter().all(|j| j["why"] == "asked by the owner"), "{}", s["q"]);
    assert_eq!(s["ev"].as_array().unwrap().iter().filter(|e| e["w"] == "desk").count(), 4, "{}", s["ev"]);
}

/// Multi-milestone project orchestrator flow: Front Desk -> Architect -> M1 Builder & Tester
/// -> pause_each wait -> Continue -> M2 Builder, Design Fixer & Tester -> consolidated epic PR.
#[tokio::test(flavor = "multi_thread")]
async fn multi_milestone_project_orchestrator() {
    let t = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("e2e-{}-proj", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let rm = t.join("remotes");
    let fk = t.join("ghd");
    std::fs::create_dir_all(rm.join("o")).unwrap();
    std::fs::create_dir_all(&fk).unwrap();
    let seed = t.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("README.md"), "# Counter App\n").unwrap();
    sh(&seed, "git init -q -b main . && git -c user.name=t -c user.email=t@t add -A && git -c user.name=t -c user.email=t@t commit -q -m init");
    sh(&rm.join("o"), &format!("git clone -q --bare \"{}\" app.git", seed.display().to_string().replace('\\', "/")));
    std::fs::write(fk.join("issues.json"), "[]").unwrap();

    let td = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let gw = wrap(&t, "gh", &td.join("fake-gh.py"));
    let cw = wrap(&t, "claude", &td.join("fake-claude.py"));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let data = t.join("data");
    let exe = env!("CARGO_BIN_EXE_apollo-office");
    let envs = [
        ("APOLLO_OFFICE_GH", gw.display().to_string()),
        ("APOLLO_OFFICE_CLAUDE", cw.display().to_string()),
        ("APOLLO_OFFICE_REMOTE", furl(&rm)),
        ("APOLLO_OFFICE_FAST", "1".into()),
        ("FAKE_GH_DIR", fk.display().to_string()),
        ("FAKE_GH_REPO", "o/app".into()),
    ];
    let _k = K(Command::new(exe).args(["--data", &data.display().to_string(), "--port", &port.to_string(), "--quiet"]).envs(envs.iter().cloned()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let l = Command::new(exe).args(["--data", &data.display().to_string(), "login-link"]).output().unwrap();
    let l = String::from_utf8_lossy(&l.stdout).lines().find(|x| x.starts_with("http")).unwrap().trim().to_string();
    let o = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = c.post(&l).header("origin", &o).send().await.unwrap();
    let ck = r.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    let get = |p: &str| {
        let (c, u, ck) = (c.clone(), format!("{o}{p}"), ck.clone());
        async move { c.get(u).header("cookie", ck).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let post = |p: &str, b: Value| {
        let (c, u, ck, o) = (c.clone(), format!("{o}{p}"), ck.clone(), o.clone());
        async move { c.post(u).header("cookie", ck).header("origin", o).header("x-ao", "1").json(&b).send().await.unwrap().json::<Value>().await.unwrap() }
    };

    assert_eq!(post("/api/host", json!({"op": "set", "id": "local", "dk": false, "raw": true, "ln": 1})).await["ok"], true);
    assert_eq!(post("/api/repo", json!({"op": "add", "full": "o/app"})).await["full"], "o/app");
    // Leave repo CI empty initially to confirm Architect's ci_cmd runs on M1 and backfills rp.ci.
    post("/api/repo", json!({"op": "set", "full": "o/app", "hunt": false, "iss": false})).await;
    assert_eq!(get("/api/state").await["orch"], false, "the orchestrator is off by default");
    assert_eq!(post("/api/set", json!({"orch": true})).await["set"]["orch"], true);

    let plan = post("/api/chat", json!({"msgs": [{"r": "you", "t": "Build a full project counter app with CLI and UI"}]})).await;
    assert_eq!(plan["plan"][0]["to"], "architect", "{plan}");
    assert_eq!(plan["plan"][0]["kind"], "project", "{plan}");
    let mut act = plan["plan"][0]["act"].clone();
    act["pause_each"] = json!(true);
    act["budget_usd"] = json!(25.0);

    let go = post("/api/chat/go", json!({"acts": [{"act": act}]})).await;
    assert_eq!(go["jobs"][0]["new"], true, "{go}");

    // Wait for Architect + Milestone 1 to finish and pause at `waiting_milestone`.
    let t0 = Instant::now();
    let proj = loop {
        let ps = get("/api/projects").await;
        let p = ps["projects"].as_array().and_then(|a| a.first()).cloned();
        if let Some(ref p) = p {
            if let Some(m) = p["milestones"].as_array().and_then(|ms| ms.iter().find(|x| x["st"] == "drop")) {
                let s = get("/api/state").await;
                panic!("milestone dropped early: {m}\n{}", s["ev"]);
            }
            if p["st"] == "waiting_milestone" {
                break p.clone();
            }
        }
        if t0.elapsed() >= Duration::from_secs(240) {
            let s = get("/api/state").await;
            panic!("project did not reach waiting_milestone in time: {ps}\n{}", s["ev"]);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let pid = proj["id"].as_i64().unwrap();
    let epic_br = proj["epic_branch"].as_str().unwrap().to_string();
    assert!(epic_br.starts_with("epic/"), "{epic_br}");
    let ms = proj["milestones"].as_array().unwrap();
    let m1 = ms.iter().find(|m| m["kind"] == "build" && m["milestone_idx"] == 1).unwrap();
    let m2 = ms.iter().find(|m| m["kind"] == "build" && m["milestone_idx"] == 2).unwrap();
    assert_eq!(m1["st"], "done", "{m1}");
    assert_eq!(m2["st"], "ready", "{m2}");
    let m2_id = m2["id"].as_i64().unwrap();

    // Verify M1 landed onto remote epic branch and README/plan/counter.sh are present.
    let o_plan = Command::new("git").args(["--git-dir", &rm.join("o/app.git").display().to_string(), "show", &format!("{epic_br}:.apollo/plan.md")]).output().unwrap();
    assert!(o_plan.status.success(), "{}", String::from_utf8_lossy(&o_plan.stderr));
    let o_sh = Command::new("git").args(["--git-dir", &rm.join("o/app.git").display().to_string(), "show", &format!("{epic_br}:counter.sh")]).output().unwrap();
    assert!(String::from_utf8_lossy(&o_sh.stdout).contains("count()"), "M1 did not land on {epic_br}");

    // Continue to Milestone 2 -> Design Fixer -> Tester -> consolidated epic_pr approval.
    let cont = post("/api/project", json!({"id": pid, "op": "continue"})).await;
    assert_eq!(cont["project"]["st"], "running", "{cont}");

    let t0 = Instant::now();
    let epic_id = loop {
        let s = get("/api/state").await;
        if let Some(j) = s["q"].as_array().unwrap().iter().find(|j| j["kind"] == "epic_pr" && j["st"] == "approve") {
            break j["id"].as_i64().unwrap();
        }
        if let Some(j) = s["dn"].as_array().unwrap().iter().find(|j| j["project_id"] == pid && j["st"] == "drop") {
            panic!("project job dropped before epic_pr: {j}\n{}", s["ev"]);
        }
        assert!(t0.elapsed() < Duration::from_secs(240), "epic_pr did not reach approve: {}", s["ev"]);
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let ej = get(&format!("/api/job?id={epic_id}")).await;
    assert_eq!(ej["milestone_summaries"].as_array().map(|v| v.len()), Some(2), "{ej}");
    let maj = ej["milestone_artifact_jobs"].as_array().unwrap();
    assert!(!maj.is_empty(), "expected consolidated milestone screenshots: {ej}");
    let art_resp = c.get(format!("{o}/api/artifact?job={m2_id}&file=design_after_desktop.png")).header("cookie", &ck).send().await.unwrap();
    assert_eq!(art_resp.status().as_u16(), 200);
    assert!(ej["df"].as_str().unwrap().contains("counter.sh"), "{}", ej["df"]);
    assert!(ej["df"].as_str().unwrap().contains("<title>Counter App</title>"), "{}", ej["df"]);

    let app_res = post("/api/act", json!({"a": "approve", "id": epic_id, "pt": "feat: Counter App"})).await;
    assert!(app_res["error"].is_null(), "{app_res}");

    let t0 = Instant::now();
    loop {
        let s = get("/api/state").await;
        if let Some(x) = s["dn"].as_array().unwrap().iter().find(|x| x["id"] == epic_id) {
            assert_eq!(x["st"], "done", "{x}\n{}", s["ev"]);
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(120), "epic_pr not submitted: {}", s["ev"]);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let p_done = get(&format!("/api/project?id={pid}")).await;
    assert_eq!(p_done["st"], "done", "{p_done}");
    let ps: Value = serde_json::from_str(&std::fs::read_to_string(fk.join("prs.json")).unwrap()).unwrap();
    assert_eq!(ps.as_array().unwrap().len(), 1, "{ps}");
    assert_eq!(ps[0]["base"], "main", "{ps}");
    assert_eq!(ps[0]["head"], epic_br, "{ps}");
    assert_eq!(ps[0]["title"], "feat: Counter App", "{ps}");
}
