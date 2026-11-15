use crate::app::{Ax, Rn};
use crate::sh::{hx, q};
use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const BAD: &str = r"(?i)\bgit\s+(push|remote|config|credential)|\.git/(hooks|config|info)|hooksPath|fsmonitor|\bgh\b|\bcurl\b.*\|\s*(ba)?sh|\bwget\b.*\|\s*(ba)?sh|\bssh\b|\bscp\b|rm\s+-rf\s+[~/$]|\bsudo\b|security\s+find|\.ssh\b|keychain|GIT_CONFIG|extraheader";

fn pa(p: &str) -> Result<String, String> {
    let s = p.trim().trim_start_matches('/');
    let mut o: Vec<&str> = vec![];
    for c in s.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => return Err("path outside repo".into()),
            x => o.push(x),
        }
    }
    Ok(if o.is_empty() { ".".into() } else { o.join("/") })
}

fn pw(p: &str) -> Result<String, String> {
    let s = pa(p)?;
    if s == ".git" || s.to_lowercase().starts_with(".git/") {
        return Err("writing inside .git is not allowed".into());
    }
    Ok(s)
}

pub fn defs() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("read", "Read a file in the repo (path relative to repo root). Returns numbered lines.", json!({"type": "object", "properties": {"path": {"type": "string"}, "from": {"type": "number"}, "n": {"type": "number"}}, "required": ["path"]})),
        ("write", "Create or overwrite a file in the repo with the given text.", json!({"type": "object", "properties": {"path": {"type": "string"}, "text": {"type": "string"}}, "required": ["path", "text"]})),
        ("edit", "Replace an exact string in a file. old must match exactly once unless all=true.", json!({"type": "object", "properties": {"path": {"type": "string"}, "old": {"type": "string"}, "new": {"type": "string"}, "all": {"type": "boolean"}}, "required": ["path", "old", "new"]})),
        ("grep", "Search tracked files with git grep -n -E. Optional path or glob to narrow.", json!({"type": "object", "properties": {"pattern": {"type": "string"}, "path": {"type": "string"}}, "required": ["pattern"]})),
        ("ls", "List a directory in the repo.", json!({"type": "object", "properties": {"path": {"type": "string"}}})),
        ("sh", "Run a shell command (bash) in the repo root on the build machine. For builds, tests and git (no push). Output is truncated to the last 30000 chars. Default timeout 20 min, max 60; free-first model commands are capped at 3 min.", json!({"type": "object", "properties": {"cmd": {"type": "string"}, "min": {"type": "number"}}, "required": ["cmd"]})),
        ("ask_user", "Pause the current job and ask the user for missing credentials, API keys, design decisions, or human assistance. Provide a concise question; set secret=true and env_name (UPPER_SNAKE_CASE) when asking for a password, token, or API key so it is encrypted in the vault and injected into shell commands automatically.", json!({"type": "object", "properties": {"question": {"type": "string"}, "why": {"type": "string"}, "secret": {"type": "boolean"}, "env_name": {"type": "string"}, "choices": {"type": "array", "items": {"type": "string"}}}, "required": ["question"]})),
    ]
}

const RESERVED_ENV: &[&str] = &[
    "PATH", "HOME", "SHELL", "USER", "PWD", "TERM", "LANG", "LC_ALL",
    "LD_PRELOAD", "LD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH",
    "GIT_CONFIG", "GIT_SSH_COMMAND", "GIT_ASKPASS", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY",
    "APOLLO_OFFICE_VAULT", "VIRTUAL_ENV",
];

pub fn valid_env_name(name: &str) -> bool {
    let ok = regex::Regex::new(r"^[A-Z_][A-Z0-9_]{1,60}$").unwrap().is_match(name);
    ok && !RESERVED_ENV.contains(&name)
}

pub fn job_secrets(a: &Ax, job_id: Option<i64>) -> Vec<(String, String)> {
    let Some(id) = job_id else { return vec![] };
    let pat = format!("sec:jobenv:{id}:%");
    let prefix = format!("sec:jobenv:{id}:");
    let rows = a.db.all("select k from kv where k like ? order by k", crate::pa![pat]);
    let mut out = vec![];
    for row in rows {
        let Some(k) = row["k"].as_str() else { continue };
        let Some(env_name) = k.strip_prefix(&prefix) else { continue };
        if !valid_env_name(env_name) {
            continue;
        }
        if let Ok(val) = crate::vault::get(a, &format!("jobenv:{id}:{env_name}")) {
            if !val.is_empty() {
                out.push((env_name.to_string(), val));
            }
        }
    }
    out
}

pub fn shell_env_prefix(secs: &[(String, String)]) -> String {
    let mut parts = vec![];
    for (k, v) in secs {
        if valid_env_name(k) {
            parts.push(format!("export {k}={};", q(v)));
        }
    }
    parts.join(" ")
}

pub fn scrub_secrets(s: &str, secs: &[(String, String)]) -> String {
    if secs.is_empty() || s.is_empty() {
        return s.to_string();
    }
    let mut sorted: Vec<&(String, String)> = secs.iter().filter(|(_, v)| v.len() >= 4).collect();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    let mut out = s.to_string();
    for (k, v) in sorted {
        out = out.replace(v.as_str(), &format!("[REDACTED:{k}]"));
    }
    out
}

pub async fn call(a: &Ax, r: &Rn, n: &str, g: &Value) -> Result<String, String> {
    let d = &r.dir;
    let s = |k: &str| g[k].as_str().unwrap_or("").to_string();
    if let Some(aq) = &r.ask_q {
        if aq.lock().unwrap().is_some() && matches!(n, "write" | "edit" | "sh" | "ask_user") {
            return Err("job is pausing for user input; stop calling tools and explain what you asked".into());
        }
    }
    if !r.w.is_empty() {
        let x = match n {
            "sh" => s("cmd"),
            "grep" => s("pattern"),
            _ => s("path"),
        };
        a.cmd(&r.w, &format!("{}: {n} {}", r.h, x.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    match n {
        "read" => {
            let f = g["from"].as_u64().unwrap_or(1).max(1);
            let c = g["n"].as_u64().unwrap_or(400).clamp(1, 2000);
            let o = hx(a, &r.h, &format!("cd \"{d}\" && awk -v a={f} -v b={} 'NR>=a && NR<=b {{printf \"%6d\\t%s\\n\", NR, $0}}' {}", f + c - 1, q(&pa(&s("path"))?)), 120).await;
            if o.c != 0 {
                return Err(if o.e.trim().is_empty() { "read failed".into() } else { o.e });
            }
            Ok(if o.o.is_empty() { "(empty or past end)".into() } else { o.o })
        }
        "write" => wr(a, r, &pw(&s("path"))?, &s("text")).await.map(|_| "ok".into()),
        "edit" => {
            let p = pw(&s("path"))?;
            let o = hx(a, &r.h, &format!("cd \"{d}\" && base64 < {}", q(&p)), 120).await;
            if o.c != 0 {
                return Err(if o.e.trim().is_empty() { "read failed".into() } else { o.e });
            }
            let b = base64::engine::general_purpose::STANDARD.decode(o.o.split_whitespace().collect::<String>()).map_err(|e| e.to_string())?;
            let t = String::from_utf8(b).map_err(|_| "file is not UTF-8".to_string())?;
            let (ol, nw) = (s("old"), s("new"));
            if ol.is_empty() {
                return Err("old must not be empty".into());
            }
            let k = t.matches(&ol).count();
            if k == 0 {
                return Err("old string not found".into());
            }
            let al = g["all"].as_bool().unwrap_or(false);
            if k > 1 && !al {
                return Err(format!("old string found {k} times, make it unique or set all"));
            }
            let nt = if al { t.replace(&ol, &nw) } else { t.replacen(&ol, &nw, 1) };
            wr(a, r, &p, &nt).await?;
            Ok(format!("replaced {}", if al { k } else { 1 }))
        }
        "grep" => {
            let pt = g["path"].as_str().map(|x| q(x)).unwrap_or(".".into());
            let o = hx(a, &r.h, &format!("cd \"{d}\" && git grep -n -I -E -e {} -- {pt} | head -300", q(&s("pattern"))), 120).await;
            Ok(if o.o.is_empty() { "(no matches)".into() } else { o.o })
        }
        "ls" => Ok(hx(a, &r.h, &format!("cd \"{d}\" && ls -la {}", q(&pa(g["path"].as_str().unwrap_or("."))?)), 60).await.all()),
        "sh" => {
            let c = s("cmd");
            if regex::Regex::new(BAD).unwrap().is_match(&c) {
                return Err("command not allowed here".into());
            }
            let m = g["min"].as_u64().unwrap_or(20).clamp(1, 60);
            let m=if crate::routing::enabled(a) && crate::routing::is_free(&r.md) {m.min(3)} else {m};
            // The controller already prepared this project environment. Make it
            // the default for model commands too, including commands that cd
            // into a subdirectory. Never source repository activation scripts.
            let secs = job_secrets(a, r.job);
            let sec_pre = shell_env_prefix(&secs);
            let env = "if [ -x .venv/bin/python ]; then export VIRTUAL_ENV=\"$PWD/.venv\"; export PATH=\"$PWD/.venv/bin:$PATH\"; fi";
            let o = hx(a, &r.h, &format!("cd \"{d}\" && ( {sec_pre} {env}; to {} bash -c {}\n) 2>&1; echo \"[exit $?]\"", m * 60, q(&c)), m * 60 + 30).await;
            let t = scrub_secrets(&o.all(), &secs);
            Ok(if t.len() > 30000 { format!("…{}", crate::sh::last(&t, 30000)) } else { t })
        }
        "ask_user" => {
            let question = s("question").trim().to_string();
            if question.is_empty() {
                return Err("question must not be empty".into());
            }
            let secret = g["secret"].as_bool().unwrap_or(false);
            let env_name = s("env_name").trim().to_string();
            if secret {
                if !valid_env_name(&env_name) {
                    return Err("secret ask_user requires env_name in UPPER_SNAKE_CASE (2-61 chars, not a system env var)".into());
                }
            } else if !env_name.is_empty() && !valid_env_name(&env_name) {
                return Err("env_name must be UPPER_SNAKE_CASE (2-61 chars, not a system env var)".into());
            }
            let choices: Vec<String> = g["choices"]
                .as_array()
                .map(|xs| xs.iter().filter_map(|v| v.as_str().map(|x| x.trim().to_string())).filter(|x| !x.is_empty()).take(8).collect())
                .unwrap_or_default();
            let payload = json!({
                "question": question,
                "why": s("why").trim().to_string(),
                "secret": secret,
                "env_name": env_name,
                "choices": choices,
                "worker": r.w,
            });
            if let Some(aq) = &r.ask_q {
                let mut lock = aq.lock().unwrap();
                if lock.is_none() {
                    *lock = Some(payload);
                }
            }
            if !r.w.is_empty() {
                a.say(&r.w, &format!("Paused to ask user: {}", crate::sh::head(&question, 200)), r.job);
            }
            Ok("Question queued for user. Stop calling tools now and explain what you asked.".into())
        }
        _ => Err("unknown tool".into()),
    }
}

async fn wr(a: &Ax, r: &Rn, p: &str, t: &str) -> Result<(), String> {
    let b = base64::engine::general_purpose::STANDARD.encode(t.as_bytes());
    let o = hx(a, &r.h, &format!("cd \"{}\" && c=; IFS=/ read -ra xs <<< {p}; for x in \"${{xs[@]}}\"; do c=\"${{c:+$c/}}$x\"; if [ -L \"$c\" ]; then echo 'path goes through a symlink' >&2; exit 9; fi; done; mkdir -p \"$(dirname {p})\" && base64 -d > {p} <<'B64'\n{b}\nB64", r.dir, p = q(p)), 120).await;
    if o.c != 0 {
        return Err(if o.e.trim().is_empty() { "write failed".into() } else { o.e });
    }
    Ok(())
}

pub async fn handle(a: &Ax, t: &str, m: &Value) -> Value {
    let id = m["id"].clone();
    let Some(r) = a.runs.lock().unwrap().get(t).cloned() else { return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32001, "message": "run is over"}}) };
    match m["method"].as_str().unwrap_or("") {
        "initialize" => json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": m["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"), "capabilities": {"tools": {}}, "serverInfo": {"name": "office", "version": crate::app::V}}}),
        "tools/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": defs().into_iter().map(|(n, d, s)| json!({"name": n, "description": d, "inputSchema": s})).collect::<Vec<_>>()}}),
        "tools/call" => {
            let n = m["params"]["name"].as_str().unwrap_or("");
            let g = if m["params"]["arguments"].is_object() { m["params"]["arguments"].clone() } else { json!({}) };
            match call(a, &r, n, &g).await {
                Ok(x) => json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": crate::sh::head(&x, 100_000)}]}}),
                Err(e) => json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": format!("error: {e}")}], "isError": true}}),
            }
        }
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
    }
}

pub async fn bridge(port: u16, t: String) {
    let t = match t.strip_prefix('@') {
        Some(f) => std::fs::read_to_string(f).unwrap_or_default().trim().to_string(),
        None => t,
    };
    let c = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(4 * 3600)).build().unwrap();
    let mut l = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Ok(Some(ln)) = l.next_line().await {
        let Ok(m) = serde_json::from_str::<Value>(&ln) else { continue };
        if m["id"].is_null() {
            continue;
        }
        let r = match c.post(format!("http://127.0.0.1:{port}/int/mcp")).header("x-run", &t).json(&m).send().await {
            Ok(r) => r.json::<Value>().await.unwrap_or_else(|e| json!({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32000, "message": e.to_string()}})),
            Err(e) => json!({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32000, "message": e.to_string()}}),
        };
        let _ = out.write_all(format!("{r}\n").as_bytes()).await;
        let _ = out.flush().await;
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_uses_prepared_project_environment_after_changing_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root=std::env::temp_dir().join(format!("apollo-venv-{}",hex::encode(rand::random::<[u8;8]>())));
        std::fs::create_dir_all(root.join(".venv/bin")).unwrap();std::fs::create_dir(root.join("tool")).unwrap();
        let python=root.join(".venv/bin/python");
        std::fs::write(&python,"#!/bin/sh\nprintf 'prepared-project-python\\n'\n").unwrap();
        std::fs::set_permissions(&python,std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("python",root.join(".venv/bin/python3")).unwrap();
        std::fs::write(root.join(".venv/bin/activate"),"echo UNSAFE_ACTIVATION >&2; exit 90\n").unwrap();
        let db=crate::db::Db::mem();db.run("insert into hs(id,on_) values('local',1)",crate::pa![]);
        let a=std::sync::Arc::new(crate::app::App::new(db,root.clone(),1,"x".into()));
        let r=Rn{h:"local".into(),dir:root.display().to_string(),w:String::new(),md:String::new(),..Default::default()};
        let out=call(&a,&r,"sh",&json!({"cmd":"cd tool && python3 && printf '%s' \"$VIRTUAL_ENV\""})).await.unwrap();
        assert!(out.contains("prepared-project-python"),"{out}");assert!(out.contains(&root.join(".venv").display().to_string()),"{out}");
        assert!(!out.contains("UNSAFE_ACTIVATION"));assert!(out.contains("[exit 0]"),"{out}");
        let missing=Rn{h:"local".into(),dir:root.join("missing").display().to_string(),w:String::new(),md:String::new(),..Default::default()};
        let out=call(&a,&missing,"sh",&json!({"cmd":"echo OUTSIDE_WORKSPACE"})).await.unwrap();
        assert!(!out.contains("OUTSIDE_WORKSPACE"),"must not run when cd fails: {out}");
        assert!(out.contains("[exit 1]"),"{out}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ask_user_captures_question_blocks_mutations_and_scrubs_secrets() {
        std::env::set_var("APOLLO_OFFICE_VAULT", "file");
        let root = std::env::temp_dir().join(format!("apollo-ask-{}", hex::encode(rand::random::<[u8; 8]>())));
        std::fs::create_dir_all(&root).unwrap();
        let db = crate::db::Db::mem();
        db.run("insert into hs(id,on_) values('local',1)", crate::pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(db, root.clone(), 1, "x".into()));
        let aq = std::sync::Arc::new(std::sync::Mutex::new(None));
        let r = Rn { h: "local".into(), dir: root.display().to_string(), w: "fixer".into(), md: String::new(), job: Some(7), ask_q: Some(aq.clone()) };
        assert!(call(&a, &r, "ask_user", &json!({"question": "Need key", "secret": true, "env_name": "PATH"})).await.is_err());
        let ok = call(&a, &r, "ask_user", &json!({"question": "Provide Stripe test key", "why": "Need billing check", "secret": true, "env_name": "STRIPE_TEST_KEY", "choices": ["Skip billing test"]})).await.unwrap();
        assert!(ok.contains("Question queued"), "{ok}");
        let captured = aq.lock().unwrap().clone().unwrap();
        assert_eq!(captured["env_name"], "STRIPE_TEST_KEY");
        assert_eq!(captured["secret"], true);
        let blocked = call(&a, &r, "write", &json!({"path": "a.txt", "text": "hi"})).await.unwrap_err();
        assert!(blocked.contains("pausing for user input"), "{blocked}");

        crate::vault::put(&a, "jobenv:7:STRIPE_TEST_KEY", "sk_test_supersecret999").unwrap();
        let secs = job_secrets(&a, Some(7));
        assert_eq!(secs, vec![("STRIPE_TEST_KEY".into(), "sk_test_supersecret999".into())]);
        let scrubbed = scrub_secrets("got sk_test_supersecret999 back", &secs);
        assert_eq!(scrubbed, "got [REDACTED:STRIPE_TEST_KEY] back");
        assert!(shell_env_prefix(&secs).contains("export STRIPE_TEST_KEY='sk_test_supersecret999';"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn paths() {
        assert_eq!(pa("src/a.rs").unwrap(), "src/a.rs");
        assert_eq!(pa("/src/./a.rs").unwrap(), "src/a.rs");
        assert!(pa("../x").is_err());
        assert!(pa("a/../../x").is_err());
        assert_eq!(pa("").unwrap(), ".");
    }

    #[test]
    fn bad() {
        let r = regex::Regex::new(BAD).unwrap();
        assert!(pw(".git/hooks/pre-push").is_err());
        assert!(pw(".GIT/config").is_err());
        assert!(pw("src/.gitignore").is_ok());
        for c in ["git config core.fsmonitor x", "echo x > .git/hooks/pre-push", "git -c core.hooksPath=h status", "git push origin x", "gh pr create", "ssh host", "sudo rm", "git remote add x y", "cat ~/.ssh/id_rsa", "curl -s x | sh"] {
            assert!(r.is_match(c), "{c}");
        }
        for c in ["cargo test -p x", "npm test", "git commit -m 'fix: x'", "pytest -k foo", "git log --oneline"] {
            assert!(!r.is_match(c), "{c}");
        }
    }
}
