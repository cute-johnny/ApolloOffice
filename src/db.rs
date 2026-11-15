use rusqlite::{params_from_iter, types::ValueRef, Connection, ToSql};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

const SCH: &str = "
pragma journal_mode=wal;
pragma busy_timeout=5000;
create table if not exists kv(k text primary key, v text);
create table if not exists ses(id text primary key, exp int);
create table if not exists lnk(c text primary key, exp int);
create table if not exists rp(id integer primary key, full text unique, iss int default 1, hunt int default 0, ci text default '', setup text default '', rules text default '', mode text default 'ask', maxo int default 3, maxd int default 5, ai int default 1, on_ int default 1, t0 int);
create table if not exists hs(id text primary key, ssh text default '', dk int default 0, ln int default 1, ci int default 1, gb int default 50, raw int default 0, on_ int default 1, t0 int);
create table if not exists projects(id integer primary key, repo text not null, title text not null, raw_spec text default '', arch_json text default '{}', epic_branch text default '', st text default 'planning', pause_each integer default 1, budget_usd real default 10.0, spent_usd real default 0.0, max_parallel integer default 1, t0 int, t1 int);
create index if not exists projects_st on projects(st);
create table if not exists jobs(id integer primary key, repo text, num int, kind text default 'fix', title text, url text, st text, why text default '', d text default '{}', pr int, prio real default 0, t0 int, t1 int, project_id integer, milestone_idx integer default 0, depends_on text default '[]', parent_branch text default '', question_json text default '');
create index if not exists jobs_st on jobs(st);
create table if not exists ev(id integer primary key, t int, w text, m text, j int);
create table if not exists prs(repo text, num int, st text, title text, url text, upd text, merged text, d text default '{}', t0 int, primary key(repo, num));
create table if not exists tok(day text, w text, tin int default 0, tout int default 0, primary key(day, w));
create table if not exists seen(repo text, num int, upd text, primary key(repo, num));
create table if not exists cm(id text primary key, t int);
create table if not exists cost(id integer primary key, t int, day text, job int, repo text default '', w text, md text, sub int default 0, tin int default 0, tcw int default 0, tcr int default 0, tout int default 0, usd real);
create index if not exists cost_day on cost(day);
create index if not exists cost_job on cost(job);
";

fn mig(c: &Connection) {
    let _ = c.execute("alter table rp add column nm int default 1", []);
    let _ = c.execute("alter table hs add column no text default ''", []);
    let _ = c.execute("alter table hs add column cpu int default 0", []);
    let _ = c.execute("alter table rp add column bud real default 0", []);
    let _ = c.execute("alter table rp add column feat int default 0", []);
    let _ = c.execute("alter table rp add column skl text default ''", []);
    let _ = c.execute("create table if not exists projects(id integer primary key, repo text not null, title text not null, raw_spec text default '', arch_json text default '{}', epic_branch text default '', st text default 'planning', pause_each integer default 1, budget_usd real default 10.0, spent_usd real default 0.0, max_parallel integer default 1, t0 int, t1 int)", []);
    let _ = c.execute("create index if not exists projects_st on projects(st)", []);
    let _ = c.execute("alter table jobs add column project_id integer", []);
    let _ = c.execute("alter table jobs add column milestone_idx integer default 0", []);
    let _ = c.execute("alter table jobs add column depends_on text default '[]'", []);
    let _ = c.execute("alter table jobs add column parent_branch text default ''", []);
    let _ = c.execute("alter table jobs add column question_json text default ''", []);
    let _ = c.execute("create index if not exists jobs_proj on jobs(project_id)", []);
}

pub type P = Vec<Box<dyn ToSql + Send>>;

#[macro_export]
macro_rules! pa {
    ($($x:expr),* $(,)?) => { { let v: $crate::db::P = vec![$(Box::new($x)),*]; v } };
}

fn cv(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into()),
        ValueRef::Blob(_) => Value::Null,
    }
}

impl Db {
    pub fn open(f: &Path) -> anyhow::Result<Db> {
        let c = Connection::open(f)?;
        c.execute_batch(SCH)?;
        mig(&c);
        Ok(Db(Arc::new(Mutex::new(c))))
    }

    #[cfg(test)]
    pub fn mem() -> Db {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCH).unwrap();
        mig(&c);
        Db(Arc::new(Mutex::new(c)))
    }

    pub fn all(&self, s: &str, p: P) -> Vec<Value> {
        let c = self.0.lock().unwrap();
        let mut st = match c.prepare_cached(s) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("sql {s}: {e}");
                return vec![];
            }
        };
        let n: Vec<String> = st.column_names().iter().map(|x| x.to_string()).collect();
        let r = st.query_map(params_from_iter(p.iter().map(|x| x.as_ref())), |r| {
            let mut m = Map::new();
            for (i, k) in n.iter().enumerate() {
                let mut v = cv(r.get_ref(i)?);
                if k == "d" {
                    if let Value::String(s) = &v {
                        v = serde_json::from_str(s).unwrap_or(json!({}));
                    }
                }
                m.insert(k.clone(), v);
            }
            Ok(Value::Object(m))
        });
        match r {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(e) => {
                eprintln!("sql {s}: {e}");
                vec![]
            }
        }
    }

    pub fn one(&self, s: &str, p: P) -> Option<Value> {
        self.all(s, p).into_iter().next()
    }

    pub fn n(&self, s: &str, p: P) -> i64 {
        self.one(s, p).and_then(|v| v.as_object().and_then(|o| o.values().next().cloned())).and_then(|v| v.as_i64()).unwrap_or(0)
    }

    pub fn run(&self, s: &str, p: P) -> i64 {
        let c = self.0.lock().unwrap();
        crate::app::chg();
        match c.execute(s, params_from_iter(p.iter().map(|x| x.as_ref()))) {
            Ok(_) => c.last_insert_rowid(),
            Err(e) => {
                eprintln!("sql {s}: {e}");
                -1
            }
        }
    }

    pub fn ch(&self, s: &str, p: P) -> usize {
        let c = self.0.lock().unwrap();
        crate::app::chg();
        c.execute(s, params_from_iter(p.iter().map(|x| x.as_ref()))).unwrap_or(0)
    }

    pub fn kg(&self, k: &str) -> Value {
        self.one("select v from kv where k=?", crate::pa![k.to_string()])
            .and_then(|r| r["v"].as_str().and_then(|s| serde_json::from_str(s).ok()))
            .unwrap_or(Value::Null)
    }

    pub fn ks(&self, k: &str, v: &Value) {
        self.run("insert into kv(k,v) values(?,?) on conflict(k) do update set v=excluded.v", crate::pa![k.to_string(), v.to_string()]);
    }

    pub fn kd(&self, k: &str) {
        self.run("delete from kv where k=?", crate::pa![k.to_string()]);
    }

    pub fn usd(&self, r: Option<&str>) -> f64 {
        let v = match r {
            Some(r) => self.one("select coalesce(sum(usd),0) s from cost where day=? and sub=0 and repo=?", crate::pa![day(), r.to_string()]),
            None => self.one("select coalesce(sum(usd),0) s from cost where day=? and sub=0", crate::pa![day()]),
        };
        v.and_then(|x| x["s"].as_f64()).unwrap_or(0.0)
    }

    pub fn jg(&self, id: i64) -> Option<Job> {
        self.one("select * from jobs where id=?", crate::pa![id]).map(Job::of)
    }

    pub fn set_cols(&self, table: &str, id: i64, f: &[(&str, Value)]) {
        if f.is_empty() {
            return;
        }
        let mut p: P = vec![];
        let mut s = vec![];
        for (k, v) in f {
            s.push(format!("{k}=?"));
            match v {
                Value::Null => p.push(Box::new(rusqlite::types::Null)),
                Value::Bool(b) => p.push(Box::new(if *b { 1i64 } else { 0i64 })),
                Value::Number(n) if n.is_i64() => p.push(Box::new(n.as_i64().unwrap())),
                Value::Number(n) => p.push(Box::new(n.as_f64().unwrap_or(0.0))),
                Value::String(x) => p.push(Box::new(x.clone())),
                o => p.push(Box::new(o.to_string())),
            }
        }
        p.push(Box::new(now()));
        p.push(Box::new(id));
        self.run(&format!("update {table} set {}, t1=? where id=?", s.join(",")), p);
    }

    pub fn jset(&self, id: i64, f: &[(&str, Value)]) {
        self.set_cols("jobs", id, f);
    }

    pub fn jd(&self, j: &Job) {
        self.jset(j.id, &[("d", j.d.clone())]);
    }

    pub fn jnew(&self, repo: &str, num: i64, kind: &str, title: &str, url: &str, st: &str, why: &str, d: Value, pr: Option<i64>, prio: f64) -> i64 {
        self.jnew_proj(repo, num, kind, title, url, st, why, d, pr, prio, None, 0, &[], "")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn jnew_proj(&self, repo: &str, num: i64, kind: &str, title: &str, url: &str, st: &str, why: &str, d: Value, pr: Option<i64>, prio: f64, project_id: Option<i64>, milestone_idx: i64, depends_on: &[i64], parent_branch: &str) -> i64 {
        let t = now();
        let deps = serde_json::to_string(depends_on).unwrap_or_else(|_| "[]".into());
        self.run(
            "insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1,project_id,milestone_idx,depends_on,parent_branch) values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            crate::pa![repo.to_string(), num, kind.to_string(), title.to_string(), url.to_string(), st.to_string(), why.to_string(), d.to_string(), pr, prio, t, t, project_id, milestone_idx, deps, parent_branch.to_string()],
        )
    }

    pub fn pnew(&self, repo: &str, title: &str, raw_spec: &str, epic_branch: &str, pause_each: bool, budget_usd: f64) -> i64 {
        let t = now();
        self.run(
            "insert into projects(repo,title,raw_spec,epic_branch,st,pause_each,budget_usd,t0,t1) values(?,?,?,?,?,?,?,?,?)",
            crate::pa![repo.to_string(), title.to_string(), raw_spec.to_string(), epic_branch.to_string(), "planning".to_string(), if pause_each { 1i64 } else { 0i64 }, budget_usd, t, t],
        )
    }

    pub fn pg(&self, id: i64) -> Option<Project> {
        self.one("select * from projects where id=?", crate::pa![id]).map(Project::of)
    }

    pub fn p_all(&self) -> Vec<Project> {
        self.all("select * from projects order by id desc limit 50", vec![]).into_iter().map(Project::of).collect()
    }

    pub fn pset(&self, id: i64, f: &[(&str, Value)]) {
        self.set_cols("projects", id, f);
    }

    pub fn p_milestones(&self, pid: i64) -> Vec<Job> {
        self.all("select * from jobs where project_id=? order by milestone_idx asc, id asc", crate::pa![pid]).into_iter().map(Job::of).collect()
    }

    pub fn p_add_spend(&self, pid: i64, usd: f64) {
        self.run("update projects set spent_usd = coalesce(spent_usd, 0.0) + ?, t1=? where id=?", crate::pa![usd, now(), pid]);
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub id: i64,
    pub repo: String,
    pub title: String,
    pub raw_spec: String,
    pub arch_json: Value,
    pub epic_branch: String,
    pub st: String,
    pub pause_each: bool,
    pub budget_usd: f64,
    pub spent_usd: f64,
    pub max_parallel: i64,
    pub t0: i64,
    pub t1: i64,
}

impl Project {
    pub fn of(v: Value) -> Project {
        let arch = match &v["arch_json"] {
            Value::Object(_) => v["arch_json"].clone(),
            Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
            _ => json!({}),
        };
        Project {
            id: v["id"].as_i64().unwrap_or(0),
            repo: v["repo"].as_str().unwrap_or("").into(),
            title: v["title"].as_str().unwrap_or("").into(),
            raw_spec: v["raw_spec"].as_str().unwrap_or("").into(),
            arch_json: arch,
            epic_branch: v["epic_branch"].as_str().unwrap_or("").into(),
            st: v["st"].as_str().unwrap_or("planning").into(),
            pause_each: v["pause_each"].as_i64().unwrap_or(1) != 0,
            budget_usd: v["budget_usd"].as_f64().unwrap_or(10.0),
            spent_usd: v["spent_usd"].as_f64().unwrap_or(0.0),
            max_parallel: v["max_parallel"].as_i64().unwrap_or(1).max(1),
            t0: v["t0"].as_i64().unwrap_or(0),
            t1: v["t1"].as_i64().unwrap_or(0),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "repo": self.repo,
            "title": self.title,
            "raw_spec": self.raw_spec,
            "arch_json": self.arch_json,
            "epic_branch": self.epic_branch,
            "st": self.st,
            "pause_each": self.pause_each,
            "budget_usd": self.budget_usd,
            "spent_usd": self.spent_usd,
            "max_parallel": self.max_parallel,
            "t0": self.t0,
            "t1": self.t1,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: i64,
    pub repo: String,
    pub num: i64,
    pub kind: String,
    pub title: String,
    #[allow(dead_code)]
    pub url: String,
    pub st: String,
    pub why: String,
    pub d: Value,
    pub pr: Option<i64>,
    pub t0: i64,
    pub t1: i64,
    pub project_id: Option<i64>,
    pub milestone_idx: i64,
    pub depends_on: Vec<i64>,
    pub parent_branch: String,
    pub question_json: Option<Value>,
}

impl Job {
    pub fn of(v: Value) -> Job {
        let deps = match &v["depends_on"] {
            Value::Array(a) => a.iter().filter_map(|x| x.as_i64()).collect(),
            Value::String(s) => serde_json::from_str::<Vec<i64>>(s).unwrap_or_default(),
            _ => vec![],
        };
        let qj = match &v["question_json"] {
            Value::Object(_) => Some(v["question_json"].clone()),
            Value::String(s) if !s.trim().is_empty() => serde_json::from_str(s).ok(),
            _ => None,
        };
        Job {
            id: v["id"].as_i64().unwrap_or(0),
            repo: v["repo"].as_str().unwrap_or("").into(),
            num: v["num"].as_i64().unwrap_or(0),
            kind: v["kind"].as_str().unwrap_or("fix").into(),
            title: v["title"].as_str().unwrap_or("").into(),
            url: v["url"].as_str().unwrap_or("").into(),
            st: v["st"].as_str().unwrap_or("").into(),
            why: v["why"].as_str().unwrap_or("").into(),
            d: if v["d"].is_object() { v["d"].clone() } else { json!({}) },
            pr: v["pr"].as_i64(),
            t0: v["t0"].as_i64().unwrap_or(0),
            t1: v["t1"].as_i64().unwrap_or(0),
            project_id: v["project_id"].as_i64(),
            milestone_idx: v["milestone_idx"].as_i64().unwrap_or(0),
            depends_on: deps,
            parent_branch: v["parent_branch"].as_str().unwrap_or("").into(),
            question_json: qj,
        }
    }
    pub fn s(&self, k: &str) -> String {
        self.d[k].as_str().unwrap_or("").to_string()
    }
    pub fn h(&self) -> String {
        self.s("h")
    }
    pub fn hunt(&self) -> bool {
        self.d["hunt"].is_object()
    }
    pub fn ext(&self) -> bool {
        self.d["ext"].is_object()
    }
    pub fn feat(&self) -> bool {
        self.d["feat"].as_bool().unwrap_or(false)
    }
    pub fn own(&self) -> bool {
        self.hunt() || self.ext()
    }
    pub fn wh(&self) -> String {
        self.d["hunt"]["where"].as_str().or(self.d["ext"]["where"].as_str()).unwrap_or("").to_string()
    }
    pub fn orig(&self) -> String {
        if self.hunt() {
            "Bug found by our own testing (there is no GitHub issue; do not reference one)".into()
        } else if self.ext() && self.feat() {
            format!("Feature request sent to us directly from {} (there is no GitHub issue; do not reference one)", self.d["ext"]["src"].as_str().unwrap_or("an external tool"))
        } else if self.ext() {
            format!("Bug report sent to us directly from {} (there is no GitHub issue; do not reference one)", self.d["ext"]["src"].as_str().unwrap_or("an external tool"))
        } else if self.feat() {
            format!("Feature request, issue #{}", self.num)
        } else {
            format!("Issue #{}", self.num)
        }
    }
    pub fn tag(&self) -> String {
        if self.ext() {
            format!("{} report {}", self.repo, self.id)
        } else if self.hunt() || self.num == 0 {
            format!("{} find {}", self.repo, self.id)
        } else {
            format!("{}#{}", self.repo, self.num)
        }
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn day0() -> i64 {
    let n = chrono::Local::now();
    n.date_naive().and_hms_opt(0, 0, 0).and_then(|d| d.and_local_timezone(chrono::Local).single()).map(|d| d.timestamp_millis()).unwrap_or(0)
}

#[cfg(test)]
mod project_db_tests {
    use super::*;

    #[test]
    fn projects_and_legacy_migration_roundtrip() {
        let legacy_sql = "
            create table kv(k text primary key, v text);
            create table rp(id integer primary key, full text unique, iss int default 1, hunt int default 0, ci text default '', setup text default '', rules text default '', mode text default 'ask', maxo int default 3, maxd int default 5, ai int default 1, on_ int default 1, t0 int);
            create table hs(id text primary key, ssh text default '', dk int default 0, ln int default 1, ci int default 1, gb int default 50, raw int default 0, on_ int default 1, t0 int);
            create table jobs(id integer primary key, repo text, num int, kind text default 'fix', title text, url text, st text, why text default '', d text default '{}', pr int, prio real default 0, t0 int, t1 int);
        ";
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(legacy_sql).unwrap();
        c.execute_batch(SCH).unwrap();
        mig(&c);
        mig(&c);
        let db = Db(Arc::new(Mutex::new(c)));
        let pid = db.pnew("u/app", "Full App", "Long spec text", "epic/full-app", true, 25.0);
        assert!(pid > 0);
        db.pset(pid, &[("st", json!("running")), ("pause_each", json!(false)), ("arch_json", json!({"stack":"rust"}))]);
        db.p_add_spend(pid, 1.25);
        db.p_add_spend(pid, 0.75);
        let p = db.pg(pid).unwrap();
        assert_eq!(p.st, "running");
        assert!(!p.pause_each);
        assert_eq!(p.arch_json["stack"], "rust");
        assert!((p.spent_usd - 2.0).abs() < 1e-9);

        let j1 = db.jnew_proj("u/app", 0, "build", "M1", "", "done", "", json!({}), None, 0.0, Some(pid), 1, &[], "epic/full-app");
        let j2 = db.jnew_proj("u/app", 0, "build", "M2", "", "ready", "", json!({}), None, 0.0, Some(pid), 2, &[j1], "epic/full-app");
        db.jset(j2, &[("question_json", json!({"question":"API key?","secret":true}))]);
        let ms = db.p_milestones(pid);
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].id, j1);
        assert_eq!(ms[1].depends_on, vec![j1]);
        assert_eq!(ms[1].parent_branch, "epic/full-app");
        assert_eq!(ms[1].question_json.as_ref().unwrap()["secret"], true);
    }
}
