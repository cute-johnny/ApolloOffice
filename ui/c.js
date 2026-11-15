// Front desk chat: the owner describes what they want in plain language,
// POST /api/chat lets a model turn it into a plan (see src/desk.rs). The
// plan shows as a card; nothing starts until the owner presses Start, which
// sends the steps they kept to POST /api/chat/go. The conversation lives in
// localStorage; the server keeps no chat state.
(() => {
  const A = window.AO, { post, esc, $ } = A;
  const KEY = "ao.chat", MAX = 60;
  const TO = { vet: "Vet", repro: "Repro", hunter: "Hunter" };
  const EX = ["Look at issue #12 and fix it", "The export command crashes on an empty file", "Add a --json flag to the list command", "Search for bugs nobody has reported yet"];
  const el = $("chat"), log = $("chL"), inp = $("chI"), send = $("chS");
  let msgs = [], busy = false;
  try { msgs = JSON.parse(localStorage.getItem(KEY) || "[]"); } catch {}
  if (!Array.isArray(msgs)) msgs = [];
  const save = () => { msgs = msgs.slice(-MAX); try { localStorage.setItem(KEY, JSON.stringify(msgs)); } catch {} };
  const who = (x) => TO[x.to] || x.to;
  const short = (r) => r.split("/")[1] || r;

  // One line per job, e.g. "→ Repro · job 12 · repo · title".
  const jrow = (j) => `<button class="cj" data-j="${j.id}" title="Open the job"><b>${j.new ? "→ " + esc(who(j)) : "Already in the office"}</b> · job ${j.id} · ${esc(short(j.repo))} · ${esc(j.title)}</button>`;

  // What starting the kept steps sets in motion, shown next to the Start button.
  const impact = (on) => {
    if (!on.length) return "Nothing is selected.";
    const ws = [...new Set(on.map(who))];
    const more = on.some((x) => x.to !== "hunter") ? " Fixer, CI and Reviewer follow on their own." : "";
    return `Starting this puts ${on.length} job${on.length > 1 ? "s" : ""} in the queue for ${ws.join(" and ")} and spends their models' budget.${more}`;
  };
  const card = (m, k) => {
    const p = m.plan;
    if (p.st === "wait") {
      const rows = p.items.map((x, i) => `<label class="cpi"><input type="checkbox" data-m="${k}" data-i="${i}"${x.on ? " checked" : ""}><span><b>→ ${esc(who(x))}</b> · ${esc(short(x.repo))} · ${esc(x.title)}${x.like ? `<em class="cpl">Looks like <a href="#" data-j="${x.like.id}">job ${x.like.id}</a>, already in the office: ${esc(x.like.title)}. Tick to start it anyway.</em>` : ""}</span></label>`).join("");
      const on = p.items.filter((x) => x.on);
      return `<div class="cp"><div class="cph">Plan · nothing has started yet</div>${rows}<div class="cpw">${esc(impact(on))}</div><div class="cpb"><button type="button" class="pri" data-go="${k}"${on.length && !busy ? "" : " disabled"}>Start${on.length > 1 ? " " + on.length : ""}</button><button type="button" data-no="${k}">Cancel</button></div></div>`;
    }
    const out = p.st === "off" ? p.items : p.items.filter((x) => !x.on);
    const why = p.st === "off" ? p.why || "not started" : "left out";
    return out.map((x) => `<div class="cn">${esc(why[0].toUpperCase() + why.slice(1))}: ${esc(who(x))} · ${esc(short(x.repo))} · ${esc(x.title)}</div>`).join("");
  };
  const bubble = (m, k) => m.r === "you"
    ? `<div class="cm you">${esc(m.t)}</div>`
    : `<div class="cm desk${m.bad ? " bad2" : ""}">${esc(m.t)}${(m.jobs || []).map(jrow).join("")}${m.plan ? card(m, k) : ""}${(m.errs || []).map((e) => `<div class="ce">Not started: ${esc(e)}</div>`).join("")}</div>`;
  const intro = () => `<div class="ci"><b>Tell the office what you need.</b><p>Describe a bug, ask for a feature, name a GitHub issue, or ask for a bug hunt. The front desk proposes a plan; once you start it, the office does the rest on its own.</p>${A.S && !A.S.nr ? `<p class="bad2">No repository is added yet. Add one under Repos first.</p>` : ""}<div class="cx">${EX.map((x) => `<button type="button">${esc(x)}</button>`).join("")}</div></div>`;
  // The plan card names the workers and the cost; the banner only reminds.
  $("chW").textContent = "He manages the whole office: his plans put work on the other workers. Nothing starts until you press Start.";
  const draw = () => {
    log.innerHTML = (msgs.length ? msgs.map(bubble).join("") : intro()) + (busy ? `<div class="cm desk wait">The front desk is thinking…</div>` : "");
    log.scrollTop = log.scrollHeight;
    send.disabled = busy;
    A.deskBusy = busy;
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
  const go = async () => {
    const t = inp.value.trim();
    if (!t || busy) return;
    msgs.push({ r: "you", t });
    const sent = msgs.filter((m) => !m.bad).map((m) => ({ r: m.r, t: said(m) }));
    // A plan still waiting is replaced by the answer to this message.
    for (const m of msgs) if (m.plan?.st === "wait") { m.plan.st = "off"; m.plan.why = "replaced by a newer message"; }
    inp.value = ""; busy = true; save(); draw();
    let r;
    try { r = await post({ msgs: sent }, "/api/chat"); } catch { r = { error: "The office is not reachable." }; }
    busy = false;
    const plan = r.plan?.length ? { st: "wait", items: r.plan.map((x) => ({ ...x, on: !x.like })) } : undefined;
    msgs.push(r.error ? { r: "desk", t: r.error, bad: 1 } : { r: "desk", t: r.reply || (plan ? "Here is my plan." : "…"), jobs: r.jobs, errs: r.errs, plan });
    save(); draw(); inp.focus();
  };

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
    else { m.plan.st = "done"; m.jobs = [...(m.jobs || []), ...(r.jobs || [])]; m.errs = [...(m.errs || []), ...(r.errs || [])]; }
    save(); draw();
  };

  const toggle = (on) => { el.hidden = !on; $("bAsk").classList.toggle("on", on); if (on) { draw(); inp.focus(); } };
  A.chat = toggle;
  $("bAsk").onclick = () => toggle(el.hidden);
  $("chX").onclick = () => toggle(false);
  $("chN").onclick = () => { if (busy) return; msgs = []; save(); draw(); inp.focus(); };
  $("chG").onclick = () => A.role && A.role("desk");
  $("chF").onsubmit = (e) => { e.preventDefault(); go(); };
  inp.onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); } };
  log.onclick = (e) => {
    const j = e.target.closest("[data-j]"), x = e.target.closest(".cx button"), g = e.target.closest("[data-go]"), n = e.target.closest("[data-no]");
    if (j) { e.preventDefault(); A.job && A.job(Number(j.dataset.j)); }
    if (x) { inp.value = x.textContent; inp.focus(); }
    if (g) start(Number(g.dataset.go));
    if (n && !busy) { const p = msgs[Number(n.dataset.no)]?.plan; if (p) { p.st = "off"; p.why = "cancelled"; save(); draw(); } }
  };
  log.onchange = (e) => {
    const c = e.target.closest("input[data-m]"), p = c && msgs[Number(c.dataset.m)]?.plan;
    if (!p || p.st !== "wait") return;
    p.items[Number(c.dataset.i)].on = c.checked;
    save();
    const at = log.scrollTop; draw(); log.scrollTop = at;
  };
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !el.hidden && $("dr").hidden) toggle(false); });
})();
