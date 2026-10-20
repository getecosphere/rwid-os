// ecOS Lite — the fallback desktop of RWID/OS (ADR-0025). The shell loads this page
// on two surfaces: `desktop` (top bar, a note) and `chrome` (dock, start menu,
// open-window chips). It speaks the shell bridge exactly like ecOS: a JSON request
// in <html data-rwid-req> + `rwid:req`, the answer in data-rwid-res + `rwid:res`.
// No network, no storage, no sign-in: only the installed native apps.
(() => {
  "use strict";
  const de = document.documentElement;
  const surface = new URLSearchParams(location.search).get("surface") === "chrome" ? "chrome" : "desktop";
  const en = !/^id\b/i.test(navigator.language || "id");
  const T = en
    ? { state: "RWID offline", title: "RWID could not be loaded", text: "You can keep working: the terminal and your apps are in the menu. RWID comes back by itself once it is reachable.", menu: "opens the menu", term: "opens the terminal", search: "Search apps", none: "No app found", start: "Menu" }
    : { state: "RWID tidak tersedia", title: "RWID belum bisa dimuat", text: "Kamu tetap bisa bekerja: terminal dan aplikasi ada di menu. RWID kembali sendiri begitu bisa dijangkau.", menu: "membuka menu", term: "membuka terminal", search: "Cari aplikasi", none: "Aplikasi tidak ditemukan", start: "Menu" };
  de.lang = en ? "en" : "id";
  de.dataset.surface = surface;
  const $ = (id) => document.getElementById(id);

  let queue = Promise.resolve();
  const ask = (req) => {
    const run = () => new Promise((resolve) => {
      const done = (v) => { document.removeEventListener("rwid:res", onRes); clearTimeout(t); resolve(v); };
      const onRes = () => {
        const raw = de.getAttribute("data-rwid-res");
        de.removeAttribute("data-rwid-res");
        try { done(JSON.parse(raw || "null")); } catch { done(null); }
      };
      const t = setTimeout(() => done(null), 3000);
      document.addEventListener("rwid:res", onRes);
      de.setAttribute("data-rwid-req", JSON.stringify(req));
      document.dispatchEvent(new Event("rwid:req"));
    });
    const p = queue.then(run, run);
    queue = p.catch(() => undefined);
    return p;
  };

  const desktop = () => {
    $("state").textContent = T.state;
    $("note-title").textContent = T.title;
    $("note-text").textContent = T.text;
    $("key-menu").textContent = T.menu;
    $("key-term").textContent = T.term;
    const tick = () => {
      $("clock").textContent = new Date().toLocaleString(de.lang, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
    };
    tick();
    setInterval(tick, 15000);
    let last = -1;
    const reserve = () => {
      const px = Math.ceil($("bar").getBoundingClientRect().bottom);
      if (px !== last) { last = px; void ask({ op: "reserve", edge: "top", px }); }
    };
    reserve();
    addEventListener("resize", reserve);
  };

  const chrome = () => {
    const menu = $("menu"), backdrop = $("backdrop"), search = $("search"), list = $("apps"), dock = $("dock");
    let apps = [], shown = [], sel = 0;
    search.placeholder = T.search;
    $("start").setAttribute("aria-label", T.start);
    $("start").title = T.start;

    // What the shell sees of this surface: the dock, or the whole screen while the menu is open.
    let lastExpanded = null, lastRects = "", lastBottom = -1;
    const report = () => {
      const expanded = !menu.hidden;
      if (expanded !== lastExpanded) { lastExpanded = expanded; void ask({ op: "chrome-size", expanded }); }
      const r = dock.getBoundingClientRect();
      const rects = expanded
        ? [[0, 0, innerWidth, innerHeight]]
        : [[Math.floor(r.left), Math.floor(r.top), Math.ceil(r.width), Math.ceil(r.height)]];
      const key = JSON.stringify(rects);
      if (key !== lastRects) { lastRects = key; void ask({ op: "input-region", rects }); }
      const bottom = Math.max(0, Math.ceil(innerHeight - r.top));
      if (bottom !== lastBottom) { lastBottom = bottom; void ask({ op: "reserve", edge: "bottom", px: bottom }); }
    };
    let raf = 0;
    const kick = () => { if (!raf) raf = requestAnimationFrame(() => { raf = 0; report(); }); };
    addEventListener("resize", kick);

    const matches = (a, q) => [a.name, a.summary, a.id, ...(a.keywords || [])].some((s) => String(s || "").toLowerCase().includes(q));
    const render = () => {
      const q = search.value.trim().toLowerCase();
      shown = q ? apps.filter((a) => matches(a, q)) : apps;
      sel = Math.min(sel, Math.max(0, shown.length - 1));
      list.replaceChildren();
      if (!shown.length) {
        const li = document.createElement("li");
        li.className = "empty";
        li.textContent = T.none;
        list.append(li);
        return;
      }
      shown.forEach((a, i) => {
        const li = document.createElement("li");
        li.setAttribute("role", "option");
        li.setAttribute("aria-selected", String(i === sel));
        let icon;
        if (typeof a.icon === "string" && a.icon.startsWith("data:image/")) {
          icon = document.createElement("img");
          icon.src = a.icon;
          icon.alt = "";
        } else {
          icon = document.createElement("span");
          icon.className = "noicon";
        }
        const text = document.createElement("span");
        text.textContent = a.name;
        if (a.summary) {
          const s = document.createElement("small");
          s.textContent = a.summary;
          text.append(s);
        }
        li.append(icon, text);
        li.addEventListener("click", () => launch(a));
        list.append(li);
      });
      list.children[sel]?.scrollIntoView({ block: "nearest" });
    };

    const close = () => { menu.hidden = true; backdrop.hidden = true; kick(); };
    const open = () => {
      search.value = "";
      sel = 0;
      render();
      menu.hidden = false;
      backdrop.hidden = false;
      kick();
      setTimeout(() => search.focus(), 0);
    };
    const toggle = () => (menu.hidden ? open() : close());
    const launch = (a) => { close(); void ask({ op: "launch", id: a.id }); };

    search.addEventListener("input", () => { sel = 0; render(); });
    search.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (shown.length) sel = (sel + (e.key === "ArrowDown" ? 1 : shown.length - 1)) % shown.length;
        render();
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (shown[sel]) launch(shown[sel]);
      } else if (e.key === "Escape") {
        close();
      }
    });
    backdrop.addEventListener("click", close);
    $("start").addEventListener("click", toggle);
    document.addEventListener("rwid:toggle-start", toggle);

    const term = $("term");
    const load = () => ask({ op: "apps" }).then((r) => {
      apps = Array.isArray(r && r.apps) ? r.apps : [];
      if (!apps.length) { setTimeout(load, 2000); return; } // shell not ready yet
      const t = apps.find((a) => /foot|terminal|console/i.test(a.id)) ||
        apps.find((a) => (a.keywords || []).some((k) => /terminal/i.test(k)));
      if (t) {
        term.textContent = t.name;
        term.hidden = false;
        term.onclick = () => void ask({ op: "launch", id: t.id });
      }
      if (!menu.hidden) render();
      kick();
    });

    const chips = $("chips");
    const setChips = (wins) => {
      if (!Array.isArray(wins)) return;
      chips.replaceChildren(...wins.map((w) => {
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = w.title;
        b.title = w.title;
        b.addEventListener("click", () => void ask({ op: "focus-window", id: w.id }));
        return b;
      }));
      kick();
    };
    document.addEventListener("rwid:windows", () => {
      try { setChips(JSON.parse(de.getAttribute("data-rwid-windows") || "[]")); } catch { /* ignore */ }
    });

    report();
    void load();
    void ask({ op: "windows" }).then(setChips);
  };

  if (surface === "chrome") chrome(); else desktop();
})();
