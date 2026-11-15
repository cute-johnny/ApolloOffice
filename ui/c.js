// Front desk chat: the owner describes what they want in plain language,
// POST /api/chat lets a model turn it into jobs and route each one to the
// right worker (see src/desk.rs). The conversation lives in localStorage;
// the server keeps no chat state.
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

  // One line per job the office started, e.g. "→ Repro · job 12 · repo · title".
  const jrow = (j) => `<button class="cj" data-j="${j.id}" title="Open the job"><b>${j.new ? "→" : "already at"} ${esc(TO[j.to] || j.to)}</b> · job ${j.id} · ${esc(j.repo.split("/")[1] || j.repo)} · ${esc(j.title)}</button>`;
  const bubble = (m) => m.r === "you"
    ? `<div class="cm you">${esc(m.t)}</div>`
    : `<div class="cm desk${m.bad ? " bad2" : ""}">${esc(m.t)}${(m.jobs || []).map(jrow).join("")}${(m.errs || []).map((e) => `<div class="ce">Not started: ${esc(e)}</div>`).join("")}</div>`;
  const intro = () => `<div class="ci"><b>Tell the office what you need.</b><p>Describe a bug, ask for a feature, name a GitHub issue, or ask for a bug hunt. The front desk hands it to the right worker and the office does the rest on its own.</p>${A.S && !A.S.nr ? `<p class="bad2">No repository is added yet. Add one under Repos first.</p>` : ""}<div class="cx">${EX.map((x) => `<button type="button">${esc(x)}</button>`).join("")}</div></div>`;
  const draw = () => {
    log.innerHTML = (msgs.length ? msgs.map(bubble).join("") : intro()) + (busy ? `<div class="cm desk wait">The front desk is thinking…</div>` : "");
    log.scrollTop = log.scrollHeight;
    send.disabled = busy;
  };

  // What the model gets back as its own earlier turns: its reply plus the
  // jobs it started, so it does not start the same work twice.
  const said = (m) => m.r === "you" ? m.t : [m.t, ...(m.jobs || []).map((j) => `[${j.new ? "started" : "already running"}: job ${j.id} at ${TO[j.to] || j.to}, ${j.repo}, ${j.title}]`), ...(m.errs || []).map((e) => `[not started: ${e}]`)].filter(Boolean).join("\n");
  const go = async () => {
    const t = inp.value.trim();
    if (!t || busy) return;
    msgs.push({ r: "you", t });
    inp.value = ""; busy = true; save(); draw();
    let r;
    try { r = await post({ msgs: msgs.filter((m) => !m.bad).map((m) => ({ r: m.r, t: said(m) })) }, "/api/chat"); } catch { r = { error: "The office is not reachable." }; }
    busy = false;
    msgs.push(r.error ? { r: "desk", t: r.error, bad: 1 } : { r: "desk", t: r.reply || (r.jobs?.length ? "Done." : "…"), jobs: r.jobs, errs: r.errs });
    save(); draw(); inp.focus();
  };

  const toggle = (on) => { el.hidden = !on; $("bAsk").classList.toggle("on", on); if (on) { draw(); inp.focus(); } };
  $("bAsk").onclick = () => toggle(el.hidden);
  $("chX").onclick = () => toggle(false);
  $("chN").onclick = () => { if (busy) return; msgs = []; save(); draw(); inp.focus(); };
  $("chG").onclick = () => A.role && A.role("desk");
  $("chF").onsubmit = (e) => { e.preventDefault(); go(); };
  inp.onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); } };
  log.onclick = (e) => {
    const j = e.target.closest(".cj"), x = e.target.closest(".cx button");
    if (j) A.job && A.job(Number(j.dataset.j));
    if (x) { inp.value = x.textContent; inp.focus(); }
  };
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !el.hidden && $("dr").hidden) toggle(false); });
})();
