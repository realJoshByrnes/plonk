// Page-side shell: the session disk (user files survive from one run to the next, not across page loads), desktop icons for
// installed programs, and a small file explorer. `host` supplies { list(), launch(path), isRunning() }.
const BS = String.fromCharCode(92);
const lower = (s) => s.toLowerCase();
const baseName = (p) => p.slice(p.lastIndexOf(BS) + 1);
const dirName = (p) => p.slice(0, Math.max(p.lastIndexOf(BS), 0));

export function programsOf(list) {
  return list.filter((e) => e.size >= 0 && /\.exe$/i.test(e.path) && !lower(e.path).startsWith("c:" + BS + "windows" + BS) && !/unwise|uninst|unins/i.test(e.path));
}

const tile = (name) => {
  const hue = [...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
  const letter = (name[0] ?? "?").toUpperCase();
  return `<svg viewBox="0 0 48 48" width="48" height="48" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${hue} 75% 68%)"/><stop offset="1" stop-color="hsl(${(hue + 40) % 360} 70% 42%)"/></linearGradient></defs><rect x="2" y="2" width="44" height="44" rx="11" fill="url(#g)"/><rect x="2" y="2" width="44" height="22" rx="11" fill="#fff" opacity=".18"/><text x="24" y="32" text-anchor="middle" font-family="system-ui,sans-serif" font-weight="700" font-size="24" fill="#fff">${letter}</text></svg>`;
};

export function createShell({ desktop, explorer, host }) {
  let desktopSig = "", expPath = "c:", selected = null;

  function renderDesktop() {
    const progs = host.isRunning() ? [] : programsOf(host.list());
    const sig = progs.map((p) => p.path).join("|");
    if (sig === desktopSig) return;
    desktopSig = sig;
    desktop.replaceChildren(...progs.slice(0, 16).map((p) => {
      const name = baseName(p.path).replace(/\.exe$/i, "");
      const el = document.createElement("div");
      el.className = "icon";
      el.title = p.path;
      el.innerHTML = tile(name) + `<span></span>`;
      el.querySelector("span").textContent = name;
      el.onclick = () => { desktop.querySelectorAll(".icon.sel").forEach((i) => i.classList.remove("sel")); el.classList.add("sel"); };
      el.ondblclick = () => host.launch(p.path);
      return el;
    }));
  }

  function renderExplorer() {
    if (explorer.hidden) return;
    const list = host.list();
    const prefix = expPath === "c:" ? "c:" + BS : expPath + BS;
    const dirs = new Map(), files = [];
    for (const e of list) {
      const p = e.path;
      if (!lower(p).startsWith(lower(prefix))) continue;
      const rest = p.slice(prefix.length);
      const cut = rest.indexOf(BS);
      if (cut >= 0) dirs.set(rest.slice(0, cut), true);
      else if (e.size < 0) dirs.set(rest, true);
      else files.push({ name: rest, size: e.size, path: p });
    }
    for (const d of ["windows", "program files", "users", "temp"]) if (expPath === "c:") dirs.set(d, true);
    explorer.querySelector(".exp-path").textContent = expPath.toUpperCase() === "C:" ? "C:" + BS : expPath;
    const body = explorer.querySelector(".exp-list");
    body.replaceChildren();
    const row = (icon, name, meta, dbl) => {
      const el = document.createElement("div");
      el.className = "exp-row";
      el.innerHTML = `<b></b><span></span><i></i>`;
      el.children[0].textContent = icon; el.children[1].textContent = name; el.children[2].textContent = meta;
      el.ondblclick = dbl;
      el.onclick = () => { body.querySelectorAll(".sel").forEach((r) => r.classList.remove("sel")); el.classList.add("sel"); };
      body.appendChild(el);
    };
    if (expPath !== "c:") row("↰", "..", "", () => { expPath = dirName(expPath) || "c:"; renderExplorer(); });
    for (const d of [...dirs.keys()].sort()) row("📁", d, "", () => { expPath = prefix + d; renderExplorer(); });
    for (const f of files.sort((a, b) => a.name.localeCompare(b.name))) {
      const exe = /\.exe$/i.test(f.name);
      row(exe ? "▶" : "📄", f.name, f.size >= 1024 ? Math.round(f.size / 1024) + " KB" : f.size + " B", exe ? () => host.launch(f.path) : () => {});
    }
  }

  explorer.querySelector(".exp-close").onclick = () => { explorer.hidden = true; };
  return {
    refresh() { renderDesktop(); renderExplorer(); },
    toggleExplorer() { explorer.hidden = !explorer.hidden; if (!explorer.hidden) renderExplorer(); },
    showExplorer(path) { if (path) expPath = path; explorer.hidden = false; renderExplorer(); },
  };
}
