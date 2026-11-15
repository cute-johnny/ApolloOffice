import json, os, re, subprocess, sys

a = sys.argv[1:]
sysp = open(a[a.index("--system-prompt-file") + 1]).read()
p = sys.stdin.read()
mc = a[a.index("--mcp-config") + 1] if "--mcp-config" in a else None
open(os.environ["FAKE_GH_DIR"] + "/claude.log", "a").write(sysp.split("\n")[0][:60] + "\n")


def emit(v):
    print(json.dumps(v), flush=True)


def ids():
    m = re.search(r"Use exactly these ids as JSON keys, in this order: (.*)$", p.strip())
    return [x.strip() for x in m.group(1).split(",")] if m else []


class M:
    def __init__(self):
        c = json.load(open(mc))["mcpServers"]["office"]
        self.p = subprocess.Popen([c["command"]] + c["args"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.n = 0
        self.rq("initialize", {"protocolVersion": "2025-06-18"})

    def rq(self, m, pa):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": m, "params": pa}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def sh(self, c):
        r = self.rq("tools/call", {"name": "sh", "arguments": {"cmd": c}})
        t = r["result"]["content"][0]["text"]
        emit({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "mcp__office__sh", "input": {"cmd": c}}]}})
        return t


emit({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 0}})
f = sysp.split("\n")[0]
if f.startswith("You are Scout"):
    r = {i: {"pick": True, "cx": "simple", "why": "clear bug"} for i in ids()}
elif f.startswith("You are Guard"):
    r = {i: {"inj": False, "why": "normal report"} for i in ids()}
elif f.startswith("You are Vet"):
    r = {"ok": True, "cx": "simple", "why": "clear bug with steps", "hint": "add.sh"}
elif f.startswith("You are Repro"):
    m = M()
    o = m.sh("bash -c 'source add.sh; add 2 3'")
    r = {"ok": "-1" in o, "how": "add 2 3 printed " + o.split("\n")[0], "where": "add.sh:1", "test": "t/add_test.sh"}
elif f.startswith("You are Fixer"):
    m = M()
    m.sh("printf '%s\\n' 'source ./add.sh' 'if [ \"$(add 2 3)\" = 5 ]; then echo \"ok 1 - adds two numbers\"; else echo \"not ok 1 - adds two numbers\"; exit 1; fi' > t/add_test.sh && git add t/add_test.sh && git commit -q -m 'test: add sums two numbers'")
    m.sh("sed -i.bak 's/\\$1 - \\$2/$1 + $2/' add.sh && rm -f add.sh.bak && git add add.sh && git commit -q -m 'fix: add sums instead of subtracting'")
    r = {"test_cmd": "bash t/add_test.sh", "tests": ["adds two numbers"], "title": "Make add sum its arguments", "cause": "add subtracted the second argument.", "fix": "Use + in add.", "untouched": ""}
elif f.startswith("You are the **Architect**"):
    r = {
        "summary": "Two-milestone Counter shell core and web UI app.",
        "stack": "POSIX shell + static HTML",
        "setup_cmd": "",
        "ci_cmd": "bash t/run.sh",
        "milestones": [
            {
                "idx": 1,
                "title": "Core counter module and test harness",
                "description": "Implement count() in counter.sh plus t/run.sh and t/core_test.sh.",
                "depends_on_idx": [],
                "visual": False,
                "acceptance": ["count 2 3 returns 5", "bash t/run.sh exits 0"],
            },
            {
                "idx": 2,
                "title": "Counter web UI page",
                "description": "Create index.html counter page and verify UI layout.",
                "depends_on_idx": [1],
                "visual": True,
                "acceptance": ["index.html renders heading", "bash t/run.sh passes"],
            },
        ],
    }
elif f.startswith("You are the **Builder**"):
    m = M()
    if "Milestone 1/" in p:
        m.sh(
            "mkdir -p t && "
            "printf '%s\\n' 'count() { echo $(( $1 + $2 )); }' > counter.sh && "
            "printf '%s\\n' 'e=0' 'for f in t/*_test.sh; do bash \"$f\" || e=1; done' 'exit $e' > t/run.sh && "
            "printf '%s\\n' 'source ./counter.sh' 'if [ \"$(count 2 3)\" = 5 ]; then echo \"ok 1 - core counter\"; else echo \"not ok 1 - core counter\"; exit 1; fi' > t/core_test.sh && "
            "git add counter.sh t/run.sh t/core_test.sh && git commit -q -m 'feat(m1): core counter module'"
        )
        r = {
            "title": "M1 core counter module",
            "test_cmd": "bash t/core_test.sh",
            "tests": ["core counter"],
            "fix": "Implemented counter.sh and test runner.",
            "untouched": "",
        }
    else:
        m.sh(
            "printf '%s\\n' '<!doctype html><html><body><h1>Counter App</h1></body></html>' > index.html && "
            "printf '%s\\n' 'if grep -q \"Counter App\" index.html; then echo \"ok 1 - renders heading\"; else echo \"not ok 1 - renders heading\"; exit 1; fi' > t/ui_test.sh && "
            "git add index.html t/ui_test.sh && git commit -q -m 'feat(m2): counter web ui'"
        )
        r = {
            "title": "M2 counter web UI",
            "test_cmd": "bash t/ui_test.sh",
            "tests": ["renders heading"],
            "fix": "Added index.html UI and smoke test.",
            "untouched": "",
        }
elif f.startswith("You are Design Fixer"):
    m = M()
    m.sh(
        "mkdir -p .apollo/artifacts && "
        "(python3 -c \"import base64; open('.apollo/artifacts/design_after_desktop.png','wb').write(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='))\" || "
        "python -c \"import base64; open('.apollo/artifacts/design_after_desktop.png','wb').write(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='))\") && "
        "printf '%s\\n' '<!doctype html><html><head><title>Counter App</title></head><body><h1>Counter App</h1></body></html>' > index.html && "
        "git add index.html && git commit -q -m 'design: polish Counter App title'"
    )
    r = {
        "ok": True,
        "summary": "Verified responsive counter layout.",
        "issues_fixed": ["Added document title and clean markup"],
        "remaining": [],
        "artifacts": ["design_after_desktop.png"],
    }
elif f.startswith("You are Tester"):
    m = M()
    m.sh(
        "mkdir -p .apollo/artifacts && "
        "(python3 -c \"import base64; open('.apollo/artifacts/test_app_home.png','wb').write(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='))\" || "
        "python -c \"import base64; open('.apollo/artifacts/test_app_home.png','wb').write(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='))\") && "
        "bash t/run.sh"
    )
    r = {
        "ok": True,
        "summary": "Verified core CLI and HTML UI end-to-end.",
        "flows_tested": ["Core counter function", "Counter HTML heading"],
        "test_cmd": "bash t/run.sh",
        "bugs_found": [],
        "artifacts": ["test_app_home.png"],
    }
elif f.startswith("You are Reviewer"):
    r = {"ok": True}
elif f.startswith("You are Watcher"):
    r = {"act": False, "change": "", "draft": ""}
elif f.startswith("You are the front desk"):
    # Routes by keywords in the owner's last message, so tests are deterministic.
    m_repo = re.search(r"Repositories in the office:\n(\S+)", p)
    repo = m_repo.group(1) if m_repo else ""
    last = p.rsplit("Owner: ", 1)[-1].strip()
    n = re.search(r"issue #(\d+)", last)
    if n:
        r = {"reply": "Vet will read the issue first.", "acts": [{"type": "issue", "repo": repo, "issue": int(n.group(1))}]}
    elif "full project" in last.lower():
        r = {"reply": "Architect will plan the full application.", "acts": [{"type": "project", "repo": repo, "title": "Counter App", "body": last}]}
    elif "hunt" in last.lower():
        r = {"reply": "Hunter will search the repository.", "acts": [{"type": "hunt", "repo": repo}]}
    elif last.lower().startswith("add "):
        r = {"reply": "Repro will plan the feature.", "acts": [{"type": "feature", "repo": repo, "title": last[:100], "body": last}]}
    elif last.endswith("?"):
        r = {"reply": "Which repository do you mean?", "acts": []}
    elif "elsewhere" in last:
        r = {"reply": "On it.", "acts": [{"type": "bug", "repo": "o/elsewhere", "title": last[:100], "body": last}]}
    else:
        r = {"reply": "Repro will reproduce the bug.", "acts": [{"type": "bug", "repo": repo, "title": last[:100], "body": last, "where": "add.sh"}]}
else:
    r = {}
emit({"type": "result", "subtype": "success", "is_error": False, "result": "```json\n" + json.dumps(r) + "\n```", "usage": {"input_tokens": 10, "output_tokens": 5}})
