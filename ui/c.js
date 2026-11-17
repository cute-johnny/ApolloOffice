// Front desk chat: the owner describes what they want in plain language,
// POST /api/chat lets a model turn it into a plan (see src/desk.rs). The
// plan shows as a card; nothing starts until the owner presses Start, which
// sends the steps they kept to POST /api/chat/go. The conversation lives in
// localStorage; the server keeps no chat state.
(() => {
  const A = window.AO, { post, esc, $ } = A;
  const KEY = "ao.chat", DKEY = "ao.draft", MAX = 60;
  const TO = { vet: "Vet", repro: "Repro", hunter: "Hunter", design_fixer: "Design Fixer", tester: "Tester", architect: "Architect" };
  const EX = [
    "Build me a complete web app from scratch",
    "Look at issue #12 and fix it",
    "Fix UI layout and dark mode spacing issues",
    "Test the app end-to-end with Playwright",
    "Search for bugs nobody has reported yet",
  ];
  const el = $("chat"), log = $("chL"), inp = $("chI"), send = $("chS");
  const needEl = $("chNeed"), att = $("chAtt"), attT = $("chAttT"), attX = $("chAttX"), fileIn = $("chFile"), upBtn = $("chUp");
  let msgs = [], draftId = localStorage.getItem(DKEY) || "", pendingSpec = null, busy = false;
  try { msgs = JSON.parse(localStorage.getItem(KEY) || "[]"); } catch {}
  if (!Array.isArray(msgs)) msgs = [];
  const save = () => {
    msgs = msgs.slice(-MAX);
    try {
      localStorage.setItem(KEY, JSON.stringify(msgs));
      if (draftId) localStorage.setItem(DKEY, draftId); else localStorage.removeItem(DKEY);
    } catch {}
  };
  const setSpec = (name, text) => {
    if (!text) { pendingSpec = null; att.hidden = true; return; }
    if (text.length > 512 * 1024) { alert("Specification exceeds the 512 KB limit."); return; }
    pendingSpec = { name: name || "spec.md", text };
    attT.textContent = `Spec attached: ${pendingSpec.name} (${(text.length / 1024).toFixed(1)} KB)`;
    att.hidden = false;
  };
  const who = (x) => TO[x.to] || x.to;
  const short = (r) => r.split("/")[1] || r;

  // One line per job, e.g. "→ Repro · job 12 · repo · title".
  // A project that already exists comes back with the project's id, not a
  // job id, so it opens the Projects drawer instead of a (wrong) job.
  const jrow = (j) => j.kind === "project" && !j.new
    ? `<button class="cj" data-pj="${j.id}" title="Open Projects"><b>Already in the office</b> · project ${j.id} · ${esc(short(j.repo))} · ${esc(j.title)}</button>`
    : `<button class="cj" data-j="${j.id}" title="Open the job"><b>${j.new ? "→ " + esc(who(j)) : "Already in the office"}</b> · job ${j.id} · ${esc(short(j.repo))} · ${esc(j.title)}</button>`;

  // What starting the kept steps sets in motion, shown next to the Start button.
  const impact = (on) => {
    if (!on.length) return "Nothing is selected.";
    const ws = [...new Set(on.map(who))];
    const hasProj = on.some((x) => x.kind === "project");
    const more = hasProj
      ? " Architect plans milestones on an epic branch, then Builders, Design Fixer, Tester and Reviewer deliver one PR."
      : on.some((x) => x.to !== "hunter")
        ? " Fixer/Design/Test/Reviewer follow automatically."
        : "";
    return `Starting this puts ${on.length} job${on.length > 1 ? "s" : ""} in the queue for ${ws.join(" and ")} and spends their models' budget.${more}`;
  };
  const card = (m, k) => {
    const p = m.plan;
    if (p.st === "wait") {
      const rows = p.items.map((x, i) => {
        const notice = x.notice ? `<div class="cpn">${esc(x.notice)}</div>` : "";
        const projOpts = x.kind === "project"
          ? `<div class="cpo"><label><input type="checkbox" data-po="${k}" data-i="${i}" data-f="pause_each"${x.act.pause_each !== false ? " checked" : ""}>Pause after each milestone</label><label>Budget $<input type="number" min="0" step="1" data-po="${k}" data-i="${i}" data-f="budget_usd" value="${Number(x.act.budget_usd ?? 10)}"></label></div>`
          : "";
        return `<label class="cpi"><input type="checkbox" data-m="${k}" data-i="${i}"${x.on ? " checked" : ""}><span><b>→ ${esc(who(x))}</b> · ${esc(short(x.repo))} · ${esc(x.title)}${notice}${projOpts}${x.like ? `<em class="cpl">Looks like <a href="#" data-j="${x.like.id}">job ${x.like.id}</a>, already in the office: ${esc(x.like.title)}. Tick to start it anyway.</em>` : ""}</span></label>`;
      }).join("");
      const on = p.items.filter((x) => x.on);
      return `<div class="cp"><div class="cph">Plan · nothing has started yet</div>${rows}<div class="cpw">${esc(impact(on))}</div><div class="cpb"><button type="button" class="pri" data-go="${k}"${on.length && !busy ? "" : " disabled"}>Start${on.length > 1 ? " " + on.length : ""}</button><button type="button" data-no="${k}">Cancel</button></div></div>`;
    }
    const out = p.st === "off" ? p.items : p.items.filter((x) => !x.on);
    const why = p.st === "off" ? p.why || "not started" : "left out";
    return out.map((x) => `<div class="cn">${esc(why[0].toUpperCase() + why.slice(1))}: ${esc(who(x))} · ${esc(short(x.repo))} · ${esc(x.title)}</div>`).join("");
  };
  const bubble = (m, k) => m.r === "you"
    ? `<div class="cm you">${esc(m.t)}${m.spec ? `<div class="cn">Attached spec: ${esc(m.spec)}</div>` : ""}</div>`
    : `<div class="cm desk${m.bad ? " bad2" : ""}">${esc(m.t)}${m.chips?.length && k === msgs.length - 1 && !busy ? `<div class="cchips">${m.chips.map((c) => `<button type="button" class="cchip">${esc(c)}</button>`).join("")}</div>` : ""}${(m.jobs || []).map(jrow).join("")}${m.plan ? card(m, k) : ""}${(m.errs || []).map((e) => `<div class="ce">Not started: ${esc(e)}</div>`).join("")}</div>`;
  const intro = () => `<div class="ci"><b>Tell the office what you need.</b><p>Ask for a brand new app (even in plain everyday words or a huge Markdown spec), report a bug, request UI design fixes, or ask Tester to verify your app.</p>${A.S && !A.S.nr ? `<p class="hint">Tip: no repo is added yet — you can add one under Repos or ask Front Desk to create a new private repo when starting a new app.</p>` : ""}<div class="cx">${EX.map((x) => `<button type="button">${esc(x)}</button>`).join("")}</div></div>`;
  // The plan card names the workers and the cost; the banner only reminds.
  $("chW").textContent = "He manages the whole office: clarifies vague requests, handles huge specs, and hands plans to Architect, Design Fixer, Tester & Repro. Nothing starts until Start.";
  const renderNeed = () => {
    if (!needEl) return;
    const wait = (A.S?.q || []).filter((j) => j.st === "need_user");
    if (!wait.length) { needEl.hidden = true; needEl.innerHTML = ""; return; }
    needEl.hidden = false;
    needEl.innerHTML = wait.map((j) => {
      let qtxt = j.why || "Waiting for your reply";
      try { const qv = typeof j.question_json === "string" ? JSON.parse(j.question_json) : j.question_json; if (qv?.question) qtxt = qv.question; } catch {}
      return `<div class="chneed-row"><span><b>Job ${j.id}:</b> ${esc(qtxt)}</span><button type="button" class="go" data-nj="${j.id}">Reply</button></div>`;
    }).join("");
  };
  // Upstream calls A.onState(s) on every poll (r.js chains it); show the
  // header buttons and the "waiting for you" strip from there.
  const sync = (s) => {
    if (!s) return;
    $("bAsk").hidden = !s.orch;
    if (!s.orch && !el.hidden) toggle(false);
    const bp = $("bProj");
    if (bp) {
      const ps = s.projects || [];
      bp.hidden = !s.orch && !ps.length;
      const act = ps.filter((p) => !["done", "drop"].includes(p.st));
      const attn = act.filter((p) => ["waiting_milestone", "need_user", "budget_reached"].includes(p.st)).length;
      bp.textContent = attn ? `Projects (${act.length} · ${attn} waiting)` : act.length ? `Projects (${act.length})` : ps.length ? `Projects (${ps.length})` : "Projects";
    }
    renderNeed();
  };
  const prevState = A.onState;
  A.onState = (s) => { prevState && prevState(s); sync(s); };
  setTimeout(() => sync(A.S), 0);
  if (needEl) needEl.onclick = (e) => { const b = e.target.closest("[data-nj]"); if (b && A.job) A.job(Number(b.dataset.nj)); };
  const draw = () => {
    log.innerHTML = (msgs.length ? msgs.map(bubble).join("") : intro()) + (busy ? `<div class="cm desk wait">The front desk is thinking…</div>` : "");
    log.scrollTop = log.scrollHeight;
    send.disabled = busy;
    A.deskBusy = busy;
    renderNeed();
  };

  // What the model gets back as its own earlier turns: its reply, the jobs
  // that started and what happened to its plan, so it does not propose the
  // same work twice and knows what the owner left out.
  const said = (m) => {
    if (m.r === "you") return m.t;
    const p = m.plan, st = p?.st;
    const plan = p ? p.items.filter((x) => st !== "done" || !x.on).map((x) => `[${st === "wait" ? "proposed, not started yet" : st === "done" ? "left out by the owner" : "not started: " + (p.why || "the owner did not start it")}: ${who(x)}, ${x.repo}, ${x.title}]`) : [];
    return [m.t, ...(m.jobs || []).map((j) => `[${j.new ? "started" : "already in the office"}: job ${j.id} at ${who(j)}, ${j.repo}, ${j.title}]`), ...plan, ...(m.errs || []).map((e) => `[not started: ${e}]`)].filter(Boolean).join("\n");
  };
  const goText = async (override) => {
    const t = (override ?? inp.value).trim();
    const spec = pendingSpec;
    if ((!t && !spec) || busy) return;
    msgs.push({ r: "you", t: t || "Build this from my attached spec.", spec: spec ? `${spec.name} (${(spec.text.length / 1024).toFixed(1)} KB)` : undefined });
    const sent = msgs.filter((m) => !m.bad).map((m) => ({ r: m.r, t: said(m) }));
    for (const m of msgs) if (m.plan?.st === "wait") { m.plan.st = "off"; m.plan.why = "replaced by a newer message"; }
    if (override == null) inp.value = "";
    setSpec(null);
    busy = true; save(); draw();
    let r;
    try { r = await post({ msgs: sent, spec: spec?.text || "", draft_id: draftId || "" }, "/api/chat"); } catch { r = { error: "The office is not reachable." }; }
    busy = false;
    if (r.draft_id) draftId = r.draft_id;
    const plan = r.plan?.length ? { st: "wait", items: r.plan.map((x) => ({ ...x, on: !x.like })) } : undefined;
    msgs.push(r.error ? { r: "desk", t: r.error, bad: 1 } : { r: "desk", t: r.reply || (plan ? "Here is my plan." : "…"), chips: r.chips, jobs: r.jobs, errs: r.errs, plan });
    save(); draw(); inp.focus();
  };
  const go = () => goText(null);

  // Start the steps the owner kept. The server checks each one again, so a
  // second press or work that arrived meanwhile makes no duplicate job.
  const start = async (k) => {
    const m = msgs[k], keep = m?.plan?.st === "wait" ? m.plan.items.filter((x) => x.on) : [];
    if (!keep.length || busy) return;
    busy = true; draw();
    let r;
    try { r = await post({ acts: keep.map((x) => ({ act: x.act, force: !!x.like })) }, "/api/chat/go"); } catch { r = { error: "The office is not reachable." }; }
    busy = false;
    if (r.error) m.errs = [...(m.errs || []), r.error];
    else {
      m.plan.st = "done";
      m.jobs = [...(m.jobs || []), ...(r.jobs || [])];
      m.errs = [...(m.errs || []), ...(r.errs || [])];
      if ((r.jobs || []).some((j) => j.kind === "project" && j.new)) draftId = "";
    }
    save(); draw();
  };

  // The chat opens only while the front desk is switched on (state `orch`).
  const toggle = (on) => { if (on && !A.S?.orch) return; const was = !el.hidden; el.hidden = !on; $("bAsk").classList.toggle("on", on); $("bAsk").setAttribute("aria-expanded", String(on)); if (on) { draw(); inp.focus(); } else if (was && el.contains(document.activeElement)) $("bAsk").focus({ preventScroll: true }); };
  A.chat = toggle;
  $("bAsk").onclick = () => toggle(el.hidden);
  if ($("bProj")) $("bProj").onclick = () => A.projects && A.projects(false);
  $("chX").onclick = () => toggle(false);
  $("chN").onclick = () => { if (busy) return; msgs = []; draftId = ""; setSpec(null); save(); draw(); inp.focus(); };
  $("chG").onclick = () => A.role && A.role("desk");
  $("chF").onsubmit = (e) => { e.preventDefault(); go(); };
  upBtn.onclick = () => fileIn.click();
  attX.onclick = () => setSpec(null);
  fileIn.onchange = async () => {
    const f = fileIn.files?.[0];
    if (f) setSpec(f.name, await f.text());
    fileIn.value = "";
  };
  el.ondragover = (e) => { e.preventDefault(); el.classList.add("drop"); };
  el.ondragleave = () => el.classList.remove("drop");
  el.ondrop = async (e) => {
    e.preventDefault(); el.classList.remove("drop");
    const f = e.dataTransfer?.files?.[0];
    if (f) setSpec(f.name, await f.text());
  };
  inp.onpaste = (e) => {
    const txt = e.clipboardData?.getData("text") || "";
    if (txt.length > 4000) { e.preventDefault(); setSpec("pasted-spec.md", txt); }
  };
  inp.onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); } };
  log.onclick = (e) => {
    const j = e.target.closest("[data-j]"), x = e.target.closest(".cx button"), ch = e.target.closest(".cchip"), g = e.target.closest("[data-go]"), n = e.target.closest("[data-no]");
    if (j) { e.preventDefault(); A.job && A.job(Number(j.dataset.j)); }
    if (e.target.closest("[data-pj]")) { e.preventDefault(); A.projects && A.projects(false); }
    if (x) { inp.value = x.textContent; inp.focus(); }
    if (ch) { goText(ch.textContent); }
    if (g) start(Number(g.dataset.go));
    if (n && !busy) { const p = msgs[Number(n.dataset.no)]?.plan; if (p) { p.st = "off"; p.why = "cancelled"; save(); draw(); } }
  };
  log.onchange = (e) => {
    const po = e.target.closest("[data-po]");
    if (po) {
      const p = msgs[Number(po.dataset.po)]?.plan;
      const item = p?.items?.[Number(po.dataset.i)];
      if (!p || p.st !== "wait" || !item) return;
      if (po.dataset.f === "pause_each") item.act.pause_each = po.checked;
      else if (po.dataset.f === "budget_usd") item.act.budget_usd = Math.max(0, Number(po.value) || 0);
      save();
      return;
    }
    const c = e.target.closest("input[data-m]"), p = c && msgs[Number(c.dataset.m)]?.plan;
    if (!p || p.st !== "wait") return;
    p.items[Number(c.dataset.i)].on = c.checked;
    save();
    const at = log.scrollTop; draw(); log.scrollTop = at;
  };
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !el.hidden && $("dr").hidden && !document.querySelector(".cmdk, .ddm") && el.contains(document.activeElement)) toggle(false); });
})();
