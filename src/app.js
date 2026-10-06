"use strict";

const TAURI = window.__TAURI__;
const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);
const listen = (event, cb) => TAURI.event.listen(event, cb);
const fileSrc = (path) => TAURI.core.convertFileSrc(path);

const $ = (sel) => document.querySelector(sel);
const body = document.body;
const els = {
  picker: $("#picker"),
  track: $("#track"),
  grid: $("#grid"),
  caption: $("#caption"),
  title: $("#title"),
  meta: $("#meta"),
  emptyActions: $("#emptyActions"),
  settings: $("#settings"),
  hints: $("#hints"),
  applyLayer: $("#applyLayer"),
  folderPath: $("#folderPath"),
  shortcutButton: $("#shortcutButton"),
  settingsError: $("#settingsError"),
  settingsBack: $("#settingsBack"),
  tabs: $("#tabs"),
  welcomeKeys: $("#welcomeKeys"),
  welcomeFolder: $("#welcomeFolder"),
  welcomeFolderHint: $("#welcomeFolderHint"),
  welcomeAutostart: $("#welcomeAutostart"),
  welcomeDone: $("#welcomeDone"),
  minimap: $("#minimap"),
};

const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

const state = {
  phase: "hidden",
  view: "picker",
  settingsFrom: null,
  snapshot: null,
  settings: null,
  items: [],
  entries: [],
  sections: [],
  flat: [],
  row: 0,
  col: null,
  gindex: 0,
  filter: "",
  anchor: null,
  status: null,
  thumbs: new Map(),
  recording: false,
  busy: false,
};

const geo = { w: 560, h: 315, small: 340, gap: 22, radius: 4, vw: 1920, vh: 1080, stageC: 486 };
const COL = { s0: 0.9, peek: 0.2, shrink: 0.075, depth: 3 };
const REVEAL_STEP = 90;

const timers = new Set();
const later = (fn, ms) => {
  const t = setTimeout(() => { timers.delete(t); fn(); }, ms);
  timers.add(t);
  return t;
};
const clearTimers = () => { timers.forEach(clearTimeout); timers.clear(); };
const nextFrame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

function prettify(name) {
  let s = name
    .replace(/[_\-.]+/g, " ")
    .replace(/\b\d{3,5}\s?[x×]\s?\d{3,5}\b/gi, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!s) s = name;
  if (s === s.toLowerCase()) s = s.charAt(0).toUpperCase() + s.slice(1);
  return s;
}

function samePath(a, b) {
  if (!a || !b) return false;
  const norm = (p) => p.replace(/\//g, "\\").toLowerCase();
  return norm(a) === norm(b);
}

const screens = () => state.snapshot?.screens || [];
const multiScreen = () => screens().length > 1;
const lockOwn = () => state.settings?.lockMode === "own";
const editingLock = () => lockOwn() && !!state.snapshot?.editingLock;
const mapOn = () => multiScreen() || (lockOwn() && screens().length > 0);
const currentPath = () => (editingLock() ? state.settings?.lockWallpaper : state.snapshot?.current);
const isCurrent = (item) => !!item && samePath(item.path, currentPath());
const editingScreen = () => screens().find((s) => s.editing) || screens().find((s) => s.here) || null;
const screenName = (s) => (!s || s.here ? "this screen" : `screen ${s.number}`);
const capitalize = (text) => text.charAt(0).toUpperCase() + text.slice(1);
const otherScreenShowing = (item) => screens().find((s) => (editingLock() || !s.editing) && samePath(s.current, item.path));

function prettyShortcut(accel) {
  return (accel || "")
    .split("+")
    .map((t) => {
      const k = t.trim();
      const low = k.toLowerCase();
      if (low === "super" || low === "cmd" || low === "command") return "Win";
      if (low === "control" || low === "ctrl") return "Ctrl";
      if (low === "alt" || low === "option") return "Alt";
      if (low === "shift") return "Shift";
      return k.length === 1 ? k.toUpperCase() : k;
    })
    .filter(Boolean);
}

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

function buildModel() {
  const q = state.filter.trim().toLowerCase();
  if (q) {
    const m = state.items.filter((i) => i.name.toLowerCase().includes(q) || i.folder.toLowerCase().includes(q));
    state.entries = m.map((item) => ({ type: "wall", key: item.path, item }));
    state.sections = m.length ? [{ name: null, items: m }] : [];
    state.flat = m;
    return;
  }
  const folders = new Map();
  const loose = [];
  for (const it of state.items) {
    if (!it.folder) { loose.push(it); continue; }
    const top = it.folder.split("/")[0];
    if (!folders.has(top)) folders.set(top, []);
    folders.get(top).push(it);
  }
  const names = [...folders.keys()].sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }));
  state.entries = [
    ...names.map((n) => ({ type: "folder", key: `folder:${n}`, name: n, items: folders.get(n) })),
    ...loose.map((item) => ({ type: "wall", key: item.path, item })),
  ];
  state.sections = [
    ...names.map((n) => ({ name: n, items: folders.get(n) })),
    ...(loose.length ? [{ name: names.length ? "Unsorted" : null, items: loose }] : []),
  ];
  state.flat = state.sections.flatMap((s) => s.items);
}

const layout = () => (state.settings?.layout === "grid" ? "grid" : "slider");
const entry = () => state.entries[state.row] || null;
const isExpanded = () => layout() === "slider" && state.col != null && entry()?.type === "folder";

function selectedItem() {
  if (layout() === "grid") return state.flat[state.gindex] || null;
  const e = entry();
  if (!e) return null;
  if (e.type === "wall") return e.item;
  return state.col != null ? e.items[state.col] || null : null;
}

function focusItem() {
  const item = selectedItem();
  if (item) return item;
  const e = entry();
  return e?.type === "folder" ? e.items[0] : null;
}

function locate(path) {
  if (!path) return null;
  for (let r = 0; r < state.entries.length; r++) {
    const e = state.entries[r];
    if (e.type === "wall" && samePath(e.item.path, path)) return { row: r, col: null };
    if (e.type === "folder") {
      const c = e.items.findIndex((i) => samePath(i.path, path));
      if (c >= 0) return { row: r, col: c };
    }
  }
  return null;
}

function focusPath(path, openFolder = false) {
  const loc = locate(path);
  state.row = loc ? loc.row : clamp(state.row, 0, Math.max(0, state.entries.length - 1));
  state.col = loc && openFolder ? loc.col : null;
  const g = state.flat.findIndex((i) => samePath(i.path, path));
  state.gindex = g >= 0 ? g : 0;
}

function rememberAnchor() {
  const f = focusItem();
  if (f) state.anchor = f.path;
}

function measure() {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const w = Math.round(Math.min(Math.max(vw * 0.34, 340), 720, vh * 0.42 * (16 / 9)));
  geo.vw = vw;
  geo.vh = vh;
  geo.w = w;
  geo.h = Math.round((w * 9) / 16);
  geo.small = Math.round(w * 0.6);
  geo.gap = Math.round(Math.max(18, w * 0.04));
  geo.stageC = vh * 0.45;
  const reach = vw / 2 - w / 2 - geo.gap;
  geo.radius = Math.max(2, Math.ceil(reach / (geo.small + geo.gap)) + 1);
  body.style.setProperty("--card-w", `${geo.w}px`);
  body.style.setProperty("--card-h", `${geo.h}px`);
  body.style.setProperty("--cap-x", `${Math.round(vw / 2 + (geo.w * COL.s0) / 2 + 40)}px`);
  body.style.setProperty("--cap-y", `${Math.round(geo.stageC)}px`);
}

function slot(d) {
  if (d === 0) return { x: 0, y: 0, s: 1, o: 1, b: 1 };
  const a = Math.abs(d);
  const x = Math.sign(d) * (geo.w / 2 + geo.gap + geo.small / 2 + (a - 1) * (geo.small + geo.gap));
  return { x, y: 0, s: geo.small / geo.w, o: a >= geo.radius ? 0 : 1, b: a === 1 ? 0.58 : a === 2 ? 0.4 : 0.3 };
}

function slotAside(d) {
  const p = slot(d);
  return { ...p, x: p.x * 1.35, o: 0, b: 0.2 };
}

function slotCol(k) {
  const h0 = geo.h * COL.s0;
  if (k === 0) return { x: 0, y: 0, s: COL.s0, o: 1, b: 1 };
  const a = Math.abs(k);
  let edge = h0 / 2;
  let sc = COL.s0;
  for (let i = 1; i <= a; i++) {
    sc = COL.s0 * (1 - COL.shrink * i);
    edge += h0 * COL.peek * Math.pow(0.82, i - 1);
  }
  const offset = edge - (geo.h * sc) / 2;
  return { x: 0, y: Math.sign(k) * offset, s: sc, o: a > COL.depth ? 0 : 1, b: a === 1 ? 0.5 : a === 2 ? 0.34 : 0.24 };
}

const thumbQueue = [];
const queued = new Set();
let thumbsActive = 0;

function wantThumb(path, urgent = false) {
  if (state.thumbs.has(path)) return;
  if (queued.has(path)) {
    if (urgent) {
      const i = thumbQueue.indexOf(path);
      if (i > 0) { thumbQueue.splice(i, 1); thumbQueue.unshift(path); }
    }
    return;
  }
  queued.add(path);
  urgent ? thumbQueue.unshift(path) : thumbQueue.push(path);
  pumpThumbs();
}

function pumpThumbs() {
  while (thumbsActive < 3 && thumbQueue.length) {
    const path = thumbQueue.shift();
    thumbsActive++;
    invoke("thumbnail", { path })
      .then((t) => state.thumbs.set(path, { src: fileSrc(t.path), color: t.color }))
      .catch((e) => state.thumbs.set(path, { error: String(e) }))
      .finally(() => {
        queued.delete(path);
        thumbsActive--;
        onThumb(path);
        pumpThumbs();
      });
  }
}

function setThumb(node, item, urgent = false) {
  node.dataset.thumb = item.path;
  node.dataset.file = item.fileName;
  const t = state.thumbs.get(item.path);
  if (!t) { wantThumb(item.path, urgent); return; }
  if (t.error) {
    if (!node.querySelector(":scope > .fallback")) node.append(el("span", "fallback", item.fileName));
    return;
  }
  let img = node.querySelector(":scope > img");
  if (!img) {
    img = el("img");
    img.alt = "";
    img.decoding = "async";
    img.draggable = false;
    img.addEventListener("load", () => img.classList.add("loaded"));
    img.addEventListener("error", () => node.append(el("span", "fallback", item.fileName)), { once: true });
    node.prepend(img);
  }
  if (img.getAttribute("src") !== t.src) img.src = t.src;
  if (img.complete && img.naturalWidth) img.classList.add("loaded");
}

function onThumb(path) {
  for (const n of document.querySelectorAll("[data-thumb]")) {
    if (n.dataset.thumb === path) setThumb(n, { path, fileName: n.dataset.file });
  }
  if (samePath(path, focusItem()?.path)) updateGlow();
}

function setDot(node, on) {
  const dot = node.querySelector(":scope > .current-dot");
  if (on && !dot) node.append(el("span", "current-dot"));
  if (!on && dot) dot.remove();
}

function glowFor(hex) {
  const n = parseInt(hex.slice(1), 16);
  const r = ((n >> 16) & 255) / 255, g = ((n >> 8) & 255) / 255, b = (n & 255) / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  let h = 0, s = 0;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
    h /= 6;
  }
  const L = Math.min(0.66, Math.max(0.5, l));
  const S = s < 0.06 ? s : Math.min(0.5, Math.max(0.2, s * 0.8));
  return `hsl(${Math.round(h * 360)} ${Math.round(S * 100)}% ${Math.round(L * 100)}%)`;
}

function updateGlow() {
  const f = focusItem();
  const t = f && state.thumbs.get(f.path);
  if (t && t.color) body.style.setProperty("--glow", glowFor(t.color));
}

let lastTitle = "";
function setTitle(text, direction = 0) {
  if (text === lastTitle) return;
  lastTitle = text;
  const old = [...els.title.querySelectorAll(".title-line:not(.leaving)")];
  const line = el("span", "title-line", text);
  const animate = direction !== 0 && state.phase === "open" && !reduceMotion.matches;
  if (animate) line.classList.add(direction > 0 ? "from-below" : "from-above");
  els.title.append(line);
  for (const o of old) {
    if (!animate) { o.remove(); continue; }
    o.classList.add("leaving", direction > 0 ? "to-above" : "to-below");
    later(() => o.remove(), 560);
  }
  if (animate) requestAnimationFrame(() => requestAnimationFrame(() => line.classList.remove("from-below", "from-above")));
}

const span = (text, cls) => el("span", cls || "", text);

function filterChip() {
  const s = el("span", "filter");
  s.innerHTML =
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><circle cx="7" cy="7" r="4.6"/><path d="m10.5 10.5 3.5 3.5" stroke-linecap="round"/></svg>';
  s.append(el("span", "filter-text", state.filter), el("span", "caret"));
  return s;
}

let capTimer = 0;
function renderCaption(direction = 0, swap = false) {
  const mode = isExpanded() ? "side" : "below";
  if (swap && state.phase === "open" && !reduceMotion.matches && body.dataset.capmode !== mode) {
    els.caption.classList.add("swap");
    clearTimeout(capTimer);
    capTimer = setTimeout(() => {
      body.dataset.capmode = isExpanded() ? "side" : "below";
      lastTitle = "";
      writeCaption(0);
      requestAnimationFrame(() => els.caption.classList.remove("swap"));
    }, 150);
    return;
  }
  clearTimeout(capTimer);
  els.caption.classList.remove("swap");
  body.dataset.capmode = mode;
  writeCaption(direction);
}

function writeCaption(direction) {
  body.classList.toggle("filtering", !!state.filter);
  body.classList.toggle("has-status", !!state.status);
  els.emptyActions.hidden = true;
  body.dataset.empty = String(!state.items.length);
  const parts = [];
  if (state.filter) parts.push(filterChip());

  if (!state.items.length) {
    setTitle("No wallpapers here yet", 0);
    const folder = state.snapshot?.folder || "your wallpaper folder";
    els.meta.replaceChildren(span(`Add images, or folders of images, to ${folder}.`));
    els.emptyActions.hidden = false;
    return;
  }
  if (!state.entries.length) {
    setTitle(`Nothing matches “${state.filter}”`, 0);
    els.meta.replaceChildren(...parts, span("Backspace to edit, Esc to clear"));
    return;
  }

  const e = entry();
  if (layout() === "slider" && e?.type === "folder" && state.col == null) {
    setTitle(e.name, direction);
    if (state.status) parts.push(span(state.status.text, state.status.kind === "error" ? "error" : "strong"));
    else {
      if (e.items.some(isCurrent)) parts.push(span("Has your current wallpaper", "strong"));
      parts.push(span(plural(e.items.length, "wallpaper")));
    }
    els.meta.replaceChildren(...parts);
    return;
  }

  const item = selectedItem();
  if (!item) return;
  setTitle(prettify(item.name), direction);
  if (state.status) {
    parts.push(span(state.status.text, state.status.kind === "error" ? "error" : "strong"));
  } else {
    const editing = editingScreen();
    const lock = editingLock();
    const elsewhere = multiScreen() && otherScreenShowing(item);
    if (lock) parts.push(span("Editing lock screen", "strong"));
    else if (multiScreen() && editing && !editing.here) parts.push(span(`Editing screen ${editing.number}`, "strong"));
    if (isCurrent(item)) {
      parts.push(span(lock ? "On the lock screen" : multiScreen() ? `On ${screenName(editing)}` : "On your desktop", "strong"));
    }
    else if (elsewhere) parts.push(span(`On screen ${elsewhere.number}`));
    if (item.width && item.height) parts.push(span(`${item.width} × ${item.height}`));
    if (isExpanded()) {
      const sub = item.folder.split("/").slice(1).join(" / ");
      if (sub) parts.push(span(sub));
      parts.push(span(`${state.col + 1} of ${e.items.length}`));
    } else if (layout() === "grid") {
      if (item.folder && state.filter) parts.push(span(item.folder.split("/").join(" / ")));
      parts.push(span(`${state.gindex + 1} of ${state.flat.length}`));
    } else if (state.filter && item.folder) {
      parts.push(span(item.folder.split("/").join(" / ")));
    }
  }
  els.meta.replaceChildren(...parts);
}

const nodes = new Map();

function place(node, p) {
  node.style.pointerEvents = p.o === 0 ? "none" : "";
  node.style.setProperty("--x", `${p.x}px`);
  node.style.setProperty("--y", `${p.y}px`);
  node.style.setProperty("--s", p.s);
  node.style.setProperty("--o", p.o);
  node.style.setProperty("--b", p.b);
}

function makeWallCard(onClick) {
  const card = el("div", "card");
  card.setAttribute("role", "option");
  card.addEventListener("click", (ev) => { ev.stopPropagation(); onClick(ev.shiftKey); });
  return card;
}

function makeFolderCard(e) {
  const card = el("div", "card folder");
  card.setAttribute("role", "option");
  card.append(el("div", "sheet s3"), el("div", "sheet s2"), el("div", "sheet s1"), el("span", "count", String(e.items.length)));
  card.addEventListener("click", (ev) => { ev.stopPropagation(); onRowClick(e.key, ev.shiftKey); });
  return card;
}

function fillFolder(card, e, urgent) {
  const sheets = [card.querySelector(".s1"), card.querySelector(".s2"), card.querySelector(".s3")];
  sheets.forEach((sheet, i) => {
    const it = e.items[i];
    sheet.style.display = it ? "" : "none";
    if (it) setThumb(sheet, it, urgent && i === 0);
  });
  setDot(sheets[0], e.items.some(isCurrent));
  card.setAttribute("aria-label", `${e.name}, ${plural(e.items.length, "wallpaper")}`);
}

let foldToken = 0;
const wait = (ms) => new Promise((resolve) => later(resolve, ms));

function renderRow(mode = null) {
  const expanded = isExpanded();
  const targets = new Map();
  const R = geo.radius;

  for (let d = -R; d <= R; d++) {
    const e = state.entries[state.row + d];
    if (!e) continue;
    const p = expanded ? (d === 0 ? slotCol(-(state.col + 1)) : slotAside(d)) : slot(d);
    let delay = 0;
    if (mode === "enter") delay = 70 + Math.abs(d) * 55;
    else if (mode === "reveal" && d !== 0) delay = 40 + (Math.abs(d) - 1) * REVEAL_STEP;
    targets.set(`r:${e.key}`, {
      d,
      p,
      z: expanded && d === 0 ? 300 - (state.col + 1) : 100 - Math.abs(d),
      selected: d === 0 && !expanded,
      delay,
      cls: mode === "reveal" ? "slow" : "",
      make: () => (e.type === "folder" ? makeFolderCard(e) : makeWallCard((shift) => onRowClick(e.key, shift))),
      fill: (n) => {
        if (e.type === "folder") fillFolder(n, e, d === 0);
        else { setThumb(n, e.item, d === 0); setDot(n, isCurrent(e.item)); }
      },
    });
  }

  if (expanded) {
    const e = entry();
    const from = Math.max(0, state.col - COL.depth - 1);
    const to = Math.min(e.items.length - 1, state.col + COL.depth + 1);
    for (let j = from; j <= to; j++) {
      const k = j - state.col;
      const item = e.items[j];
      let delay = 0;
      if (mode === "drift") delay = 90;
      else if (mode === "enter") delay = 120 + Math.abs(k) * 60;
      else if (mode === "expand") delay = 200 + Math.abs(k) * 70;
      targets.set(`c:${item.path}`, {
        k,
        p: slotCol(k),
        z: 300 - Math.abs(k),
        selected: k === 0,
        delay,
        cls: mode === "expand" ? "slow" : mode === "drift" ? "drift" : "",
        make: () => makeWallCard((shift) => onColClick(item.path, shift)),
        fill: (n) => { setThumb(n, item, k === 0); setDot(n, isCurrent(item)); },
      });
    }
  }

  const spawn = (key, t) => {
    if (key.startsWith("c:")) {
      if (mode === "expand") return { x: 0, y: 0, s: COL.s0 * 0.96, o: 0, b: 0.7 };
      if (mode === "drift") return { ...t.p, y: t.p.y - Math.round(geo.h * 0.1), o: 0 };
      return { ...slotCol(t.k + Math.sign(t.k)), o: 0 };
    }
    const next = t.d + Math.sign(t.d || 1);
    return { ...(expanded ? slotAside(next) : slot(next)), o: 0 };
  };

  reconcile(targets, mode, spawn);
  body.dataset.expanded = String(expanded);
}

function reconcile(targets, mode, spawn) {
  const entering = mode === "enter";
  for (const [key, node] of nodes) {
    if (targets.has(key)) continue;
    nodes.delete(key);
    node.style.transitionDelay = "0ms";
    node.classList.add("leaving");
    later(() => node.remove(), 650);
  }
  const fresh = [];
  for (const [key, t] of targets) {
    let node = nodes.get(key);
    if (!node) {
      node = t.make();
      node.dataset.key = key;
      nodes.set(key, node);
      els.track.append(node);
      node.classList.add("no-anim");
      if (entering) {
        node.classList.add("enter");
        place(node, t.p);
      } else {
        place(node, spawn(key, t));
        fresh.push([node, t]);
      }
    } else {
      place(node, t.p);
    }
    node.classList.remove("slow", "drift", "folding", "keep");
    if (t.cls) node.classList.add(t.cls);
    node.style.transitionDelay = `${t.delay}ms`;
    node.style.zIndex = String(t.z);
    node.setAttribute("aria-selected", String(t.selected));
    t.fill(node);
  }
  if (fresh.length) {
    void els.track.offsetWidth;
    for (const [node, t] of fresh) {
      node.classList.remove("no-anim");
      place(node, t.p);
    }
  }
}

function clearRow() {
  for (const n of nodes.values()) n.remove();
  nodes.clear();
}

function onRowClick(key, shift = false) {
  if (state.phase !== "open" || state.busy) return;
  const r = state.entries.findIndex((e) => e.key === key);
  if (r < 0) return;
  const e = state.entries[r];
  if (r !== state.row) { selectRow(r); return; }
  if (e.type === "folder") isExpanded() ? collapse() : expand();
  else applySelected(shift);
}

function onColClick(path, shift = false) {
  if (state.phase !== "open" || state.busy) return;
  const e = entry();
  const j = e?.items.findIndex((i) => i.path === path) ?? -1;
  if (j < 0) return;
  if (j === state.col) applySelected(shift);
  else selectCol(j);
}

function afterMove(direction, swap = false) {
  state.status = null;
  rememberAnchor();
  renderCaption(direction, swap);
  updateGlow();
  renderHints();
}

function selectRow(r) {
  if (!state.entries.length || state.busy) return;
  r = clamp(r, 0, state.entries.length - 1);
  if (r === state.row && state.col == null) return;
  const dir = Math.sign(r - state.row) || 1;
  if (isExpanded()) { collapse(); return; }
  state.row = r;
  state.col = null;
  renderRow(null);
  afterMove(dir);
}

function expand() {
  const e = entry();
  if (!e || e.type !== "folder" || isExpanded() || state.busy) return;
  state.col = 0;
  renderRow("expand");
  afterMove(1, true);
}

async function collapse() {
  if (!isExpanded() || state.busy) return;
  state.busy = true;
  const token = ++foldToken;
  const alive = () => token === foldToken && state.phase === "open";
  const e = entry();
  els.caption.classList.add("swap");
  const lift = Math.round(geo.h * 0.1);

  if (state.col > 0) {
    for (const [key, n] of [...nodes]) {
      if (!key.startsWith("c:")) continue;
      const j = e.items.findIndex((i) => `c:${i.path}` === key);
      const p = slotCol(j - state.col);
      nodes.delete(key);
      n.style.transitionDelay = "0ms";
      n.classList.add("drift", "leaving");
      place(n, { ...p, y: p.y + lift, o: 0 });
      later(() => n.remove(), 500);
    }
    state.col = 0;
    const folderNode = nodes.get(`r:${e.key}`);
    if (folderNode) {
      const p = slotCol(-1);
      folderNode.classList.add("no-anim");
      place(folderNode, { ...p, y: p.y - lift, o: 0 });
      void folderNode.offsetWidth;
      folderNode.classList.remove("no-anim");
    }
    renderRow("drift");
    await wait(620);
    if (!alive()) return;
  }

  const top = nodes.get(`c:${e.items[0].path}`);
  const folder = nodes.get(`r:${e.key}`);
  for (const [key, n] of nodes) {
    if (!key.startsWith("c:") || n === top) continue;
    n.classList.remove("slow", "drift");
    n.classList.add("folding");
    n.style.transitionDelay = "";
    n.style.zIndex = String(Math.max(1, parseInt(n.style.zIndex || "300", 10) - 240));
    place(n, { x: 0, y: geo.h * 0.04, s: COL.s0 * 0.95, o: 0, b: 0.5 });
  }
  if (folder) {
    folder.classList.remove("slow", "drift");
    folder.classList.add("folding", "keep");
    folder.style.transitionDelay = "0ms";
    folder.style.zIndex = "90";
    place(folder, slot(0));
  }
  if (top) {
    top.classList.remove("slow", "drift");
    top.classList.add("folding", "keep");
    top.style.transitionDelay = "0ms";
    top.style.zIndex = "100";
    place(top, { x: 0, y: 0, s: 1, o: 1, b: 1 });
  }
  await wait(540);
  if (!alive()) return;
  if (top) top.classList.add("leaving");
  if (folder) folder.classList.remove("folding", "keep");

  state.col = null;
  state.busy = false;
  renderRow("reveal");
  afterMove(-1, true);
}

function selectCol(j) {
  const e = entry();
  if (!isExpanded() || state.busy) return;
  j = clamp(j, 0, e.items.length - 1);
  if (j === state.col) return;
  const dir = Math.sign(j - state.col);
  state.col = j;
  renderRow(null);
  afterMove(dir);
}

const gridEls = new Map();
let gridObserver = null;

function renderGrid(entering = false) {
  if (!gridObserver) {
    gridObserver = new IntersectionObserver(
      (list) => {
        for (const e of list) {
          if (!e.isIntersecting) continue;
          const item = state.flat[Number(e.target.dataset.index)];
          if (item) setThumb(e.target, item);
        }
      },
      { root: els.grid, rootMargin: "300px 0px" }
    );
  }
  gridObserver.disconnect();
  gridEls.clear();
  const frag = document.createDocumentFragment();
  let i = 0;
  let delayIndex = 0;
  for (const sec of state.sections) {
    if (sec.name) {
      const h = el("h3", "grid-heading", sec.name);
      h.append(span(plural(sec.items.length, "wallpaper")));
      if (entering) { h.classList.add("enter"); h.style.transitionDelay = `${60 + Math.min(delayIndex, 20) * 24}ms`; }
      frag.append(h);
    }
    for (const item of sec.items) {
      const idx = i++;
      const tile = el("div", "tile");
      tile.setAttribute("role", "option");
      tile.dataset.index = String(idx);
      tile.setAttribute("aria-selected", String(idx === state.gindex));
      if (isCurrent(item)) tile.append(el("span", "current-dot"));
      if (state.thumbs.has(item.path)) setThumb(tile, item);
      if (entering) {
        tile.classList.add("no-anim", "enter");
        tile.style.transitionDelay = `${60 + Math.min(delayIndex, 20) * 24}ms`;
      }
      delayIndex++;
      tile.addEventListener("click", (ev) => {
        ev.stopPropagation();
        if (state.phase !== "open") return;
        if (idx === state.gindex) applySelected(ev.shiftKey);
        else selectGrid(idx);
      });
      gridEls.set(item.path, tile);
      frag.append(tile);
    }
  }
  els.grid.replaceChildren(frag);
  for (const tile of gridEls.values()) gridObserver.observe(tile);
  const cur = gridEls.get(state.flat[state.gindex]?.path);
  if (cur) cur.scrollIntoView({ block: "center" });
}

function selectGrid(i) {
  if (!state.flat.length) return;
  i = clamp(i, 0, state.flat.length - 1);
  if (i === state.gindex) return;
  const dir = Math.sign(i - state.gindex);
  state.gindex = i;
  for (const tile of gridEls.values()) tile.setAttribute("aria-selected", String(Number(tile.dataset.index) === i));
  const cur = gridEls.get(state.flat[i].path);
  if (cur) {
    setThumb(cur, state.flat[i], true);
    cur.scrollIntoView({ block: "nearest", behavior: reduceMotion.matches ? "auto" : "smooth" });
  }
  afterMove(dir);
}

function gridVertical(dir) {
  const cur = gridEls.get(state.flat[state.gindex]?.path);
  if (!cur) return;
  const top = cur.offsetTop;
  const cx = cur.offsetLeft + cur.offsetWidth / 2;
  const tiles = [...gridEls.values()];
  let rowTop = null;
  for (const t of tiles) {
    const tt = t.offsetTop;
    if (dir > 0 ? tt > top + 4 : tt < top - 4) {
      if (rowTop === null || (dir > 0 ? tt < rowTop : tt > rowTop)) rowTop = tt;
    }
  }
  if (rowTop === null) return;
  let best = null;
  let bestDist = Infinity;
  for (const t of tiles) {
    if (Math.abs(t.offsetTop - rowTop) > 4) continue;
    const dist = Math.abs(t.offsetLeft + t.offsetWidth / 2 - cx);
    if (dist < bestDist) { bestDist = dist; best = t; }
  }
  if (best) selectGrid(Number(best.dataset.index));
}

function renderPicker(mode = null) {
  body.dataset.layout = layout();
  if (layout() === "slider") {
    clearRow();
    renderRow(mode === "enter" ? "enter" : null);
    if (mode !== "enter") fadeInRow();
  } else {
    body.dataset.expanded = "false";
    renderGrid(mode === "enter");
  }
  renderCaption(0);
  updateGlow();
  renderHints();
  renderMinimap();
}

function fadeInRow() {
  for (const n of nodes.values()) { n.classList.add("no-anim"); n.style.opacity = "0"; }
  requestAnimationFrame(() => {
    for (const n of nodes.values()) { n.classList.remove("no-anim"); n.style.opacity = ""; }
  });
}

function applyFilter() {
  rememberAnchor();
  buildModel();
  if (state.filter) {
    const i = state.entries.findIndex((e) => samePath(e.key, state.anchor));
    state.row = Math.max(0, i);
    state.col = null;
    state.gindex = Math.max(0, i);
  } else {
    focusPath(state.anchor);
  }
  state.status = null;
  renderPicker();
}

function hint(keys, label, optional = false) {
  const h = el("span", optional ? "hint optional" : "hint");
  const k = el("span", "keys");
  for (const key of keys) k.append(el("kbd", null, key));
  h.append(k, el("span", null, label));
  return h;
}

function renderHints() {
  if (state.view === "welcome") {
    els.hints.replaceChildren(hint(["Enter"], state.items.length ? "Show wallpapers" : "Done"), hint(["Esc"], "Close"));
    return;
  }
  if (state.view === "settings") {
    els.hints.replaceChildren(hint(["Ctrl", "Tab"], "Next tab"), hint(["Esc"], state.settingsFrom === "picker" ? "Back" : "Close"));
    return;
  }
  if (!state.items.length) {
    els.hints.replaceChildren(hint(["Ctrl", ","], "Settings"), hint(["Esc"], "Close"));
    return;
  }
  const esc = hint(["Esc"], state.filter ? "Clear filter" : isExpanded() ? "Close folder" : "Close");
  const multi = multiScreen();
  const oneByDefault = !!state.settings?.enterThisScreen;
  const editing = editingScreen();
  const lock = editingLock();
  const setHint = lock
    ? hint(["Enter"], "Set lock screen")
    : multi
      ? hint(["Enter"], oneByDefault ? `Set on ${screenName(editing)}` : "Set on all screens")
      : hint(["Enter"], "Set wallpaper");
  const flipHint = multi && !lock
    ? hint(["Shift", "Enter"], oneByDefault ? "All screens" : `${capitalize(screenName(editing))} only`)
    : null;
  const screenHint = mapTiles().length > 1
    ? hint(["Ctrl", ...mapArrows()], multi ? "Other screen" : "Lock screen", true)
    : null;
  const settingsHint = hint(["Ctrl", ","], "Settings");
  const uiHint = hint(["Ctrl", "H"], uiHintLabel());
  if (layout() === "grid") {
    els.hints.replaceChildren(
      ...[
        hint(["←", "↑", "↓", "→"], "Browse"),
        setHint,
        flipHint,
        screenHint,
        hint(["Tab"], "Row view", true),
        hint(["A–Z"], "Filter", true),
        settingsHint,
        uiHint,
        esc,
      ].filter(Boolean)
    );
    return;
  }
  if (isExpanded()) {
    els.hints.replaceChildren(
      ...[
        hint(["↑", "↓"], "Browse folder"),
        setHint,
        flipHint,
        screenHint,
        hint(["Tab"], "Grid view", true),
        settingsHint,
        uiHint,
        esc,
      ].filter(Boolean)
    );
    return;
  }
  const folder = entry()?.type === "folder";
  els.hints.replaceChildren(
    ...[
      hint(["←", "→"], "Browse"),
      folder ? hint(["↓"], "Open folder") : setHint,
      folder ? null : flipHint,
      screenHint,
      hint(["Tab"], "Grid view", true),
      hint(["A–Z"], "Filter", true),
      settingsHint,
      uiHint,
      esc,
    ].filter(Boolean)
  );
}

function applySnapshot(snap) {
  state.snapshot = snap;
  state.settings = {
    enterThisScreen: false,
    followScreen: true,
    mapSize: "medium",
    lockMode: "off",
    lockSpot: "left",
    editGlow: true,
    uiHidden: false,
    hideHints: true,
    hideTitle: false,
    hideDetails: false,
    hideMap: false,
    ...snap.settings,
  };
  body.dataset.dim = snap.settings.dim || "soft";
  body.dataset.layout = layout();
  body.dataset.screens = (snap.screens || []).length > 1 ? "many" : "one";
  applyUiAttributes();
  renderSettings();
}

async function loadLibrary() {
  const [snap, items] = await Promise.all([invoke("get_state"), invoke("list_wallpapers")]);
  applySnapshot(snap);
  state.items = items;
  buildModel();
  const start = locate(snap.current) ? snap.current : state.anchor;
  focusPath(start);
  const f = focusItem();
  if (f) wantThumb(f.path, true);
}

let enterToken = 0;

async function enterPicker() {
  const token = ++enterToken;
  const stillHere = () => token === enterToken && state.phase !== "hidden" && state.phase !== "closing";
  els.caption.classList.add("enter");
  els.hints.classList.add("enter");
  renderPicker("enter");
  await nextFrame();
  if (!stillHere()) return;
  const entering = [...nodes.values(), ...gridEls.values(), ...els.grid.querySelectorAll(".grid-heading")];
  for (const n of entering) n.classList.remove("no-anim");
  await nextFrame();
  if (!stillHere()) return;
  for (const n of entering) n.classList.remove("enter");
  later(() => els.caption.classList.remove("enter"), 140);
  later(() => els.hints.classList.remove("enter"), 320);
  later(() => { for (const n of entering) n.style.transitionDelay = "0ms"; }, 520);
}

async function open(view = "picker") {
  if (state.phase === "open" || state.phase === "opening") {
    if (view === "settings" && state.view === "picker") showSettings("picker");
    return;
  }
  if (state.phase === "applying") return;
  clearTimers();
  state.busy = false;
  state.phase = "opening";
  state.filter = "";
  state.status = null;
  state.settingsFrom = null;
  state.lockImage = undefined;
  lastTitle = "";
  els.title.replaceChildren();
  els.applyLayer.className = "apply-layer";
  els.applyLayer.replaceChildren();

  try {
    await loadLibrary();
  } catch (e) {
    state.items = [];
    buildModel();
    state.status = { kind: "error", text: String(e) };
  }
  if (state.snapshot?.firstRun) view = "welcome";
  state.view = view;
  if (view !== "welcome") await waitForThumb(focusItem()?.path, 260);
  if (state.phase !== "opening") return;

  measure();
  body.dataset.view = view;
  body.dataset.phase = "opening";
  if (view === "welcome") {
    showWelcome();
    await nextFrame();
  } else {
    await enterPicker();
  }
  if (state.phase !== "opening") return;
  state.phase = "open";
  body.dataset.phase = "open";
  if (view === "settings") showSettings(null);
}

function showWelcome() {
  const n = state.items.length;
  els.welcomeKeys.replaceChildren(...prettyShortcut(state.settings?.shortcut).map((k) => el("kbd", null, k)));
  els.welcomeFolder.textContent = state.snapshot?.folder || "";
  els.welcomeFolder.title = state.snapshot?.folder || "";
  els.welcomeFolderHint.textContent = n
    ? `Found ${plural(n, "wallpaper")}. Folders inside it show up as stacks.`
    : "Drop your wallpapers in here. Folders inside it show up as stacks.";
  els.welcomeDone.textContent = n ? "Show my wallpapers" : "Done";
  clearRow();
  renderHints();
  requestAnimationFrame(() => els.welcomeDone.focus({ preventScroll: true }));
}

async function finishWelcome() {
  try {
    applySnapshot(await invoke("finish_welcome", { autostart: els.welcomeAutostart.checked }));
  } catch (_) {}
}

async function leaveWelcome() {
  if (state.view !== "welcome") return;
  await finishWelcome();
  if (!state.items.length) { close(); return; }
  state.view = "picker";
  body.dataset.view = "picker";
  if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
  enterPicker();
}

async function dismissWelcome() {
  if (state.view !== "welcome") return;
  await finishWelcome();
  close();
}

const ARROW_DIRECTIONS = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };

const middle = (rect) => ({ x: (rect[0] + rect[2]) / 2, y: (rect[1] + rect[3]) / 2 });

function mapTiles() {
  const list = screens();
  const tiles = list.map((s, i) => ({ kind: "screen", index: i, screen: s, rect: s.rect }));
  if (!lockOwn() || !list.length) return tiles;
  const primary = list.find((s) => s.rect[0] === 0 && s.rect[1] === 0) || list[0];
  const pw = (primary.rect[2] - primary.rect[0]) * LOCK_SCALE;
  const ph = (primary.rect[3] - primary.rect[1]) * LOCK_SCALE;
  const [left, top, right, bottom] = bounds(list.map((s) => s.rect));
  const gap = Math.max(right - left, bottom - top) * 0.06;
  const rect = state.settings?.lockSpot === "above"
    ? [right - pw, top - gap - ph, right, top - gap]
    : [left - gap - pw, bottom - ph, left - gap, bottom];
  tiles.push({ kind: "lock", rect });
  return tiles;
}

function bounds(rects) {
  return [
    Math.min(...rects.map((r) => r[0])),
    Math.min(...rects.map((r) => r[1])),
    Math.max(...rects.map((r) => r[2])),
    Math.max(...rects.map((r) => r[3])),
  ];
}

function editingTile(tiles) {
  if (editingLock()) return tiles.find((t) => t.kind === "lock");
  const s = editingScreen();
  return tiles.find((t) => t.screen === s);
}

function mapArrows() {
  const centres = mapTiles().map((t) => middle(t.rect));
  let across = false;
  let stacked = false;
  centres.forEach((a, i) => {
    for (const b of centres.slice(i + 1)) {
      if (Math.abs(a.x - b.x) >= Math.abs(a.y - b.y)) across = true;
      else stacked = true;
    }
  });
  return [...(across ? ["←"] : []), ...(stacked ? ["↑", "↓"] : []), ...(across ? ["→"] : [])];
}

function neighbor(direction) {
  const tiles = mapTiles();
  const from = editingTile(tiles);
  if (!from) return null;
  const origin = middle(from.rect);
  let best = null;
  let bestScore = Infinity;
  for (const t of tiles) {
    if (t === from) continue;
    const p = middle(t.rect);
    const dx = p.x - origin.x;
    const dy = p.y - origin.y;
    const along = { left: -dx, right: dx, up: -dy, down: dy }[direction];
    const across = direction === "left" || direction === "right" ? Math.abs(dy) : Math.abs(dx);
    if (along <= 0 || across > along * 2) continue;
    const score = along + across * 2;
    if (score < bestScore) {
      bestScore = score;
      best = t;
    }
  }
  return best;
}

function selectTile(tile) {
  if (!tile) return;
  if (tile.kind === "lock") selectLock();
  else selectScreen(tile.index);
}

const minimapNodes = [];
let lockNode = null;
let lockImageLoading = false;

const TAB_KEYS = { ArrowLeft: -1, ArrowRight: 1, Home: -1, End: 1 };
const MAP_SIZES = { small: [0.125, 0.12], medium: [0.18, 0.17], large: [0.25, 0.24] };
const LOCK_SCALE = 0.65;
const LOCK_ICON =
  '<svg viewBox="0 0 10 12" aria-hidden="true"><path d="M2.5 5V3.6a2.5 2.5 0 0 1 5 0V5h.4A1.1 1.1 0 0 1 9 6.1v4.8A1.1 1.1 0 0 1 7.9 12H2.1A1.1 1.1 0 0 1 1 10.9V6.1A1.1 1.1 0 0 1 2.1 5h.4Zm1.3 0h2.4V3.6a1.2 1.2 0 0 0-2.4 0V5Z"/></svg>';

function mapNode(cls, onClick) {
  const node = el("button", cls);
  node.type = "button";
  node.append(el("span", "map-number"));
  node.addEventListener("click", (ev) => {
    ev.stopPropagation();
    onClick(node);
  });
  els.minimap.append(node);
  return node;
}

function placeTile(node, rect, origin, scale) {
  node.style.left = `${Math.round((rect[0] - origin[0]) * scale) + 2}px`;
  node.style.top = `${Math.round((rect[1] - origin[1]) * scale) + 2}px`;
  node.style.width = `${Math.round((rect[2] - rect[0]) * scale) - 4}px`;
  node.style.height = `${Math.round((rect[3] - rect[1]) * scale) - 4}px`;
}

function showImage(node, path) {
  if (path) {
    setThumb(node, { path, fileName: "" }, true);
  } else {
    node.querySelector(":scope > img")?.remove();
    delete node.dataset.thumb;
  }
}

async function loadLockImage() {
  if (lockImageLoading) return;
  lockImageLoading = true;
  let path = null;
  try { path = await invoke("lock_screen_image"); } catch (_) {}
  lockImageLoading = false;
  if (state.lockImage === undefined) state.lockImage = path || null;
  if (lockNode) showImage(lockNode, state.lockImage || state.settings?.lockWallpaper);
}

function renderMinimap() {
  const list = screens();
  if (!mapOn()) {
    els.minimap.replaceChildren();
    minimapNodes.length = 0;
    lockNode = null;
    return;
  }
  const tiles = mapTiles();
  const [left, top, right, bottom] = bounds(tiles.map((t) => t.rect));
  const [baseLeft, baseTop, baseRight, baseBottom] = bounds(list.map((s) => s.rect));
  const width = right - left;
  const height = bottom - top;
  const size = MAP_SIZES[state.settings?.mapSize] || MAP_SIZES.medium;
  let boxW = innerWidth * size[0];
  let boxH = innerHeight * size[1];
  if (lockOwn()) {
    if (state.settings.lockSpot === "above") boxH *= Math.min(1.6, height / (baseBottom - baseTop));
    else boxW *= Math.min(1.6, width / (baseRight - baseLeft));
  }
  const scale = Math.min(boxW / width, boxH / height);
  els.minimap.dataset.size = state.settings?.mapSize || "medium";
  els.minimap.style.width = `${Math.round(width * scale)}px`;
  els.minimap.style.height = `${Math.round(height * scale)}px`;
  body.style.setProperty("--map-h", `${Math.round(height * scale)}px`);

  const lock = editingLock();
  while (minimapNodes.length > list.length) minimapNodes.pop().remove();
  list.forEach((s, i) => {
    const node = minimapNodes[i] || (minimapNodes[i] = mapNode("map-screen", (n) => selectScreen(minimapNodes.indexOf(n))));
    placeTile(node, s.rect, [left, top], scale);
    node.setAttribute("aria-current", String(!!s.editing && !lock));
    node.setAttribute("aria-label", `Screen ${s.number} (Ctrl + ${s.number})`);
    node.title = `Screen ${s.number} (Ctrl + ${s.number})`;
    node.querySelector(".map-number").textContent = s.number;
    showImage(node, s.current);
  });

  const lockTile = tiles.find((t) => t.kind === "lock");
  if (!lockTile) {
    lockNode?.remove();
    lockNode = null;
    return;
  }
  if (!lockNode) {
    lockNode = mapNode("map-screen map-lock", () => selectLock());
    lockNode.querySelector(".map-number").innerHTML = LOCK_ICON;
    lockNode.setAttribute("aria-label", "Lock screen (Ctrl + L)");
    lockNode.title = "Lock screen (Ctrl + L)";
  }
  placeTile(lockNode, lockTile.rect, [left, top], scale);
  lockNode.setAttribute("aria-current", String(lock));
  showImage(lockNode, state.lockImage || state.settings?.lockWallpaper);
  if (state.lockImage === undefined && state.phase !== "hidden") loadLockImage();
}

async function selectLock() {
  if (!lockOwn() || editingLock() || moving) return;
  if (state.phase !== "open" || state.view !== "picker" || state.busy) return;
  try {
    applySnapshot(await invoke("select_lock"));
  } catch (_) {}
  refreshScreenUi();
}

function refreshScreenUi() {
  state.status = null;
  renderMinimap();
  if (layout() === "slider") renderRow(null);
  else for (const [path, tile] of gridEls) setDot(tile, samePath(path, currentPath()));
  renderCaption(0);
  updateGlow();
  renderHints();
}

let moving = false;
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function selectScreen(index) {
  const target = screens()[index];
  if (!target || (target.editing && !editingLock()) || moving) return;
  if (state.phase !== "open" || state.view !== "picker" || state.busy) return;
  if (state.settings.followScreen === false || target.here) {
    try {
      applySnapshot(await invoke("select_screen", { index }));
    } catch (_) {}
    refreshScreenUi();
    flashEdge();
    return;
  }
  moving = true;
  rememberAnchor();
  body.classList.add("moving");
  await pause(180);
  try {
    applySnapshot(await invoke("select_screen", { index }));
  } catch (_) {}
  await nextFrame();
  await pause(60);
  if (state.phase === "open") {
    measure();
    state.status = null;
    lastTitle = "";
    body.classList.remove("moving");
    flashEdge();
    await enterPicker();
  }
  body.classList.remove("moving");
  moving = false;
}

async function openFolder() {
  if (state.view === "welcome") await finishWelcome();
  try {
    await invoke("open_folder");
    close();
  } catch (e) {
    state.status = { kind: "error", text: String(e) };
    if (state.view === "picker") renderCaption();
  }
}

function waitForThumb(path, ms) {
  if (!path || state.thumbs.has(path)) return Promise.resolve();
  return new Promise((resolve) => {
    const start = performance.now();
    const tick = () => {
      if (state.thumbs.has(path) || performance.now() - start > ms) resolve();
      else setTimeout(tick, 16);
    };
    tick();
  });
}

let closing = null;
function close() {
  if (state.phase === "hidden" || state.phase === "closing") return closing;
  stopRecording();
  clearTimers();
  clearTimeout(capTimer);
  state.busy = false;
  moving = false;
  body.classList.remove("moving");
  rememberAnchor();
  state.phase = "closing";
  body.dataset.phase = "closing";
  closing = new Promise((resolve) => {
    later(async () => {
      state.phase = "hidden";
      body.dataset.phase = "hidden";
      body.dataset.view = "picker";
      state.view = "picker";
      clearRow();
      els.grid.replaceChildren();
      gridEls.clear();
      els.caption.classList.remove("swap");
      els.applyLayer.className = "apply-layer";
      els.applyLayer.replaceChildren();
      await nextFrame();
      try { await invoke("hide_overlay"); } catch (_) {}
      closing = null;
      resolve();
    }, 240);
  });
  return closing;
}

function appliesEverywhere(shift) {
  if (!multiScreen()) return true;
  return state.settings?.enterThisScreen ? shift : !shift;
}

async function applySelected(flip = false) {
  if (state.phase !== "open" || state.view !== "picker" || state.busy || moving) return;
  const item = selectedItem();
  if (!item) { expand(); return; }
  const node =
    layout() === "grid" ? gridEls.get(item.path) : isExpanded() ? nodes.get(`c:${item.path}`) : nodes.get(`r:${item.path}`);
  const thumb = state.thumbs.get(item.path);
  state.phase = "applying";
  body.dataset.phase = "applying";
  state.status = null;

  const everywhere = appliesEverywhere(flip);
  const toLock = editingLock();
  let result = { scope: "all" };
  const request = invoke("apply_wallpaper", { path: item.path, everywhere }).then(
    (applied) => { if (applied) result = applied; return null; },
    (e) => String(e)
  );

  let anim = null;
  if (node && !reduceMotion.matches) {
    const r = node.getBoundingClientRect();
    const layer = els.applyLayer;
    const img = el("img");
    img.alt = "";
    if (thumb?.src) img.src = thumb.src;
    layer.replaceChildren(img);
    const full = new Image();
    full.src = fileSrc(item.path);
    full.decode().then(() => { img.src = full.src; }).catch(() => {});
    layer.classList.add("active");
    const radius = layout() === "slider" ? 14 * (r.width / geo.w) : 10;
    anim = layer.animate(
      [
        { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, borderRadius: `${radius}px` },
        { left: "0px", top: "0px", width: `${innerWidth}px`, height: `${innerHeight}px`, borderRadius: "0px" },
      ],
      { duration: 620, easing: "cubic-bezier(0.22, 0.8, 0.16, 1)", fill: "forwards" }
    );
    els.caption.classList.add("swap");
    later(() => { body.dataset.capmode = "below"; els.caption.classList.remove("swap"); }, 160);
  }

  const [error] = await Promise.all([request, anim ? anim.finished.catch(() => {}) : null]);

  if (error) {
    if (anim) {
      anim.reverse();
      await anim.finished.catch(() => {});
    }
    els.applyLayer.className = "apply-layer";
    els.applyLayer.replaceChildren();
    state.phase = "open";
    body.dataset.phase = "open";
    state.status = { kind: "error", text: `${toLock ? "Couldn't set the lock screen." : "Couldn't set wallpaper."} ${error}` };
    renderCaption();
    return;
  }

  if (result.scope === "lock" || result.lockScreen === true) {
    state.settings.lockWallpaper = item.path;
    state.lockImage = item.path;
  }
  if (result.scope === "lock") {
    renderMinimap();
    state.anchor = item.path;
    state.status = { kind: "info", text: "Set on the lock screen" };
    writeCaption(0);
    await pause(380);
    close();
    return;
  }
  state.snapshot.current = item.path;
  for (const s of screens()) if (result.scope === "all" || s.editing) s.current = item.path;
  renderMinimap();
  state.anchor = item.path;
  let text = !multiScreen()
    ? "Wallpaper set"
    : result.scope === "all"
      ? "Set on all screens"
      : `Set on ${screenName(editingScreen())}`;
  if (result.lockScreen === true) text = multiScreen() ? `${text} and the lock screen` : "Wallpaper and lock screen set";
  const lockFailed = result.lockScreen === false;
  if (lockFailed) text += ` · Lock screen didn't change${result.lockError ? `: ${result.lockError}` : ""}`;
  state.status = { kind: lockFailed ? "error" : "info", text };
  writeCaption(0);
  await new Promise((r) => setTimeout(r, lockFailed ? 2600 : 380));
  close();
}

function renderSettings() {
  const s = state.settings;
  if (!s) return;
  els.folderPath.textContent = state.snapshot?.folder || "Not set";
  els.folderPath.title = state.snapshot?.folder || "";
  for (const group of document.querySelectorAll(".segmented[data-setting]")) {
    for (const b of group.querySelectorAll("button")) {
      b.setAttribute("aria-checked", String(s[group.dataset.setting] === b.dataset.value));
    }
  }
  for (const select of document.querySelectorAll(".select[data-setting]")) {
    const key = select.dataset.setting;
    const options = [...select.querySelectorAll(".select-option")];
    const chosen = options.find((o) => o.dataset.value === s[key]) || options[0];
    for (const o of options) o.setAttribute("aria-selected", String(o === chosen));
    select.querySelector(".select-value").textContent = chosen.textContent;
    const note = document.querySelector(`[data-hint-for="${key}"]`);
    if (note) note.textContent = chosen.dataset.hint || "";
  }
  for (const input of document.querySelectorAll("input[data-setting]")) {
    const v = s[input.dataset.setting];
    if (input.type === "checkbox") input.checked = !!v;
    else if (document.activeElement !== input) input.value = v ?? "";
  }
  if (!state.recording) {
    els.shortcutButton.classList.remove("recording");
    els.shortcutButton.replaceChildren(...prettyShortcut(s.shortcut).map((k) => el("kbd", null, k)));
    els.shortcutButton.setAttribute("aria-label", `Shortcut: ${prettyShortcut(s.shortcut).join(" + ")}. Click to change.`);
  }
  const err = state.snapshot?.shortcutError;
  if (err && !els.settingsError.textContent) els.settingsError.textContent = err;
  if (state.view === "settings" && state.settingsTab && !visibleTabs().some((t) => t.dataset.tab === state.settingsTab)) {
    setTab("general", { instant: true });
  }
}

async function saveSettings(patch) {
  const next = { ...state.settings, ...patch };
  const folderChanged = "folder" in patch;
  try {
    const snap = await invoke("save_settings", { settings: next });
    els.settingsError.textContent = "";
    applySnapshot(snap);
  } catch (e) {
    els.settingsError.textContent = String(e);
    try { applySnapshot(await invoke("get_state")); } catch (_) {}
  }
  if (folderChanged) {
    state.filter = "";
    state.anchor = null;
    await loadLibrary().catch(() => {});
    if (state.view === "welcome") showWelcome();
    else renderPicker();
  }
}

function visibleTabs() {
  return [...els.tabs.querySelectorAll(".tab")].filter((t) => multiScreen() || !t.hasAttribute("data-multi"));
}

function setTab(name, { instant = false, focus = false } = {}) {
  const tabs = visibleTabs();
  const target = tabs.find((t) => t.dataset.tab === name) || tabs[0];
  if (state.settingsTab !== target.dataset.tab) closeSelects();
  state.settingsTab = target.dataset.tab;
  for (const t of els.tabs.querySelectorAll(".tab")) {
    const on = t === target;
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
  }
  for (const pane of document.querySelectorAll(".pane")) pane.classList.toggle("active", pane.dataset.pane === state.settingsTab);
  els.tabs.classList.toggle("instant", instant);
  els.tabs.style.setProperty("--tab-x", `${target.offsetLeft}px`);
  els.tabs.style.setProperty("--tab-w", `${target.offsetWidth}px`);
  if (instant) {
    void els.tabs.offsetWidth;
    els.tabs.classList.remove("instant");
  }
  if (focus) target.focus({ preventScroll: true });
}

function stepTab(step) {
  const tabs = visibleTabs();
  const index = Math.max(0, tabs.findIndex((t) => t.dataset.tab === state.settingsTab));
  const next = tabs[(index + step + tabs.length) % tabs.length];
  setTab(next.dataset.tab, { focus: document.activeElement?.classList.contains("tab") });
}

function showSettings(from) {
  stopRecording();
  state.view = "settings";
  state.settingsFrom = from;
  body.dataset.view = "settings";
  els.settingsBack.textContent = from === "picker" ? "Back to wallpapers" : "Done";
  els.settingsError.textContent = state.snapshot?.shortcutError || "";
  renderSettings();
  setTab(state.settingsTab || "general", { instant: true });
  renderHints();
  requestAnimationFrame(() => els.settingsBack.focus({ preventScroll: true }));
}

function leaveSettings() {
  stopRecording();
  closeSelects();
  if (state.settingsFrom === "picker") {
    state.view = "picker";
    body.dataset.view = "picker";
    if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
    renderPicker();
  } else {
    close();
  }
}

async function pickFolder() {
  const path = await invoke("pick_folder").catch(() => null);
  if (path) await saveSettings({ folder: path });
}

const CODE_NAMES = { Space: "Space", Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]", Backslash: "\\", Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/" };

function keyToken(e) {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit\d$/.test(c)) return c.slice(5);
  if (/^F\d{1,2}$/.test(c)) return c;
  if (/^Numpad\d$/.test(c)) return c;
  if (/^Arrow(Up|Down|Left|Right)$/.test(c)) return c;
  if (["Home", "End", "PageUp", "PageDown", "Insert", "Delete", "Pause", "PrintScreen"].includes(c)) return c;
  return CODE_NAMES[c] || null;
}

function startRecording() {
  state.recording = true;
  els.shortcutButton.classList.add("recording");
  els.shortcutButton.textContent = "Press a shortcut…";
}

function stopRecording() {
  if (!state.recording) return;
  state.recording = false;
  renderSettings();
}

function handleRecording(e) {
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") { stopRecording(); return; }
  if (["Control", "Alt", "Shift", "Meta", "OS", "AltGraph"].includes(e.key)) return;
  const key = keyToken(e);
  const mods = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  if (!key || !mods.length) {
    els.shortcutButton.textContent = "Add Ctrl, Alt, Shift or Win";
    return;
  }
  state.recording = false;
  saveSettings({ shortcut: [...mods, key].join("+") });
}

function uiHintLabel() {
  const s = state.settings || {};
  if (s.uiHidden) return "Show UI";
  return s.hideHints !== false && !s.hideTitle && !s.hideDetails ? "Hide hints" : "Hide UI";
}

function applyUiAttributes() {
  const s = state.settings || {};
  body.dataset.ui = s.uiHidden ? "hidden" : "shown";
  body.dataset.hideHints = String(s.hideHints !== false);
  body.dataset.hideTitle = String(!!s.hideTitle);
  body.dataset.hideDetails = String(!!s.hideDetails);
  body.dataset.hideMap = String(!!s.hideMap);
  body.dataset.map = mapOn() ? "on" : "off";
  body.dataset.lock = s.lockMode || "off";
}

function flashEdge() {
  if (state.settings?.editGlow === false || !multiScreen() || editingLock() || !editingScreen()?.here) return;
  const edge = $(".edit-glow");
  edge.classList.remove("flash");
  void edge.offsetWidth;
  edge.classList.add("flash");
}

function toggleUi() {
  state.settings.uiHidden = !state.settings.uiHidden;
  applyUiAttributes();
  renderHints();
  saveSettings({ uiHidden: state.settings.uiHidden });
}

function toggleLayout() {
  rememberAnchor();
  const next = layout() === "grid" ? "slider" : "grid";
  state.settings.layout = next;
  focusPath(state.anchor, true);
  if (next === "slider" && state.filter) state.col = null;
  measure();
  renderPicker();
  saveSettings({ layout: next });
}

function rowKeys(key, e) {
  const expanded = isExpanded();
  const isFolder = entry()?.type === "folder";
  switch (key) {
    case "ArrowRight": selectRow(state.row + 1); return true;
    case "ArrowLeft": selectRow(state.row - 1); return true;
    case "ArrowDown":
      if (expanded) selectCol(state.col + 1);
      else if (isFolder) expand();
      return true;
    case "ArrowUp":
      if (expanded) state.col > 0 ? selectCol(state.col - 1) : collapse();
      return true;
    case "PageDown": expanded ? selectCol(state.col + 5) : selectRow(state.row + 5); return true;
    case "PageUp": expanded ? selectCol(state.col - 5) : selectRow(state.row - 5); return true;
    case "Home": expanded ? selectCol(0) : selectRow(0); return true;
    case "End": expanded ? selectCol(entry().items.length - 1) : selectRow(state.entries.length - 1); return true;
  }
  return false;
}

function gridKeys(key) {
  switch (key) {
    case "ArrowRight": selectGrid(state.gindex + 1); return true;
    case "ArrowLeft": selectGrid(state.gindex - 1); return true;
    case "ArrowDown": gridVertical(1); return true;
    case "ArrowUp": gridVertical(-1); return true;
    case "PageDown": for (let i = 0; i < 3; i++) gridVertical(1); return true;
    case "PageUp": for (let i = 0; i < 3; i++) gridVertical(-1); return true;
    case "Home": selectGrid(0); return true;
    case "End": selectGrid(state.flat.length - 1); return true;
  }
  return false;
}

window.addEventListener("keydown", (e) => {
  if (state.phase === "hidden" || state.phase === "closing") return;
  if (state.recording) return handleRecording(e);
  if (state.phase === "applying") { e.preventDefault(); return; }
  if (state.busy && e.key !== "Escape") { e.preventDefault(); return; }

  if (state.view === "welcome") {
    if (e.key === "Escape") { e.preventDefault(); dismissWelcome(); }
    else if (e.key === "Enter" && !e.target.closest?.("button, input")) { e.preventDefault(); leaveWelcome(); }
    return;
  }
  if (state.view === "settings") {
    if (e.key === "Escape") {
      e.preventDefault();
      if (document.querySelector(".select.open")) closeSelects();
      else leaveSettings();
    }
    else if (e.ctrlKey && e.key === "Tab") { e.preventDefault(); closeSelects(); stepTab(e.shiftKey ? -1 : 1); }
    else if (e.target.classList?.contains("tab") && TAB_KEYS[e.key]) {
      e.preventDefault();
      const tabs = visibleTabs();
      if (e.key === "Home" || e.key === "End") setTab(tabs[e.key === "Home" ? 0 : tabs.length - 1].dataset.tab, { focus: true });
      else stepTab(TAB_KEYS[e.key]);
    }
    return;
  }

  const key = e.key;
  if ((e.ctrlKey || e.metaKey) && key === ",") { e.preventDefault(); showSettings("picker"); return; }
  if (e.ctrlKey && key.toLowerCase() === "h") { e.preventDefault(); toggleUi(); return; }
  if (e.ctrlKey && ARROW_DIRECTIONS[key]) {
    e.preventDefault();
    selectTile(neighbor(ARROW_DIRECTIONS[key]));
    return;
  }
  if (e.ctrlKey && key.toLowerCase() === "l") { e.preventDefault(); selectLock(); return; }
  if (e.ctrlKey && /^[1-9]$/.test(key)) {
    e.preventDefault();
    selectScreen(Number(key) - 1);
    return;
  }
  if (moving) { e.preventDefault(); return; }
  if (layout() === "grid" ? gridKeys(key) : rowKeys(key, e)) { e.preventDefault(); return; }

  switch (key) {
    case "Enter": e.preventDefault(); applySelected(e.shiftKey); return;
    case "Tab": e.preventDefault(); toggleLayout(); return;
    case "Escape":
      e.preventDefault();
      if (state.filter) { state.filter = ""; applyFilter(); }
      else if (isExpanded()) collapse();
      else if (!state.busy) close();
      return;
    case "Backspace":
      e.preventDefault();
      if (state.filter) {
        state.filter = e.ctrlKey ? "" : state.filter.slice(0, -1);
        applyFilter();
      } else if (isExpanded()) collapse();
      return;
  }
  if (key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey && (key !== " " || state.filter)) {
    e.preventDefault();
    state.filter += key;
    applyFilter();
  }
});

let wheelAcc = 0;
let wheelAxis = "";
let wheelLast = 0;
let wheelStep = 0;
window.addEventListener(
  "wheel",
  (e) => {
    if (state.phase !== "open" || state.view !== "picker" || layout() !== "slider") return;
    e.preventDefault();
    if (state.busy) return;
    const now = performance.now();
    const horizontal = Math.abs(e.deltaX) > Math.abs(e.deltaY);
    const axis = isExpanded() && !horizontal ? "col" : "row";
    if (now - wheelLast > 220 || axis !== wheelAxis) wheelAcc = 0;
    wheelLast = now;
    wheelAxis = axis;
    wheelAcc += horizontal ? e.deltaX : e.deltaY;
    if (Math.abs(wheelAcc) >= 60 && now - wheelStep > 80) {
      const step = Math.sign(wheelAcc);
      if (axis === "col") {
        if (step < 0 && state.col === 0) collapse();
        else selectCol(state.col + step);
      } else selectRow(state.row + step);
      wheelAcc = 0;
      wheelStep = now;
    }
  },
  { passive: false }
);

els.picker.addEventListener("mousedown", (e) => {
  if (state.phase !== "open") return;
  if (e.target.closest(".card, .tile, .caption, .minimap, button")) return;
  if (isExpanded()) collapse();
  else close();
});
els.settings.addEventListener("mousedown", (e) => {
  if (document.querySelector(".select.open")) return;
  if (state.phase === "open" && !e.target.closest(".panel")) leaveSettings();
});
$("#welcome").addEventListener("mousedown", (e) => {
  if (state.phase === "open" && !e.target.closest(".welcome-inner")) dismissWelcome();
});
els.hints.addEventListener("mousedown", (e) => e.stopPropagation());

document.addEventListener("click", (e) => {
  const a = e.target.closest("[data-action]");
  if (!a) return;
  const action = a.dataset.action;
  if (action === "pick-folder") pickFolder();
  else if (action === "open-folder") openFolder();
  else if (action === "finish-welcome") leaveWelcome();
  else if (action === "settings-back") leaveSettings();
  else if (action === "quit") invoke("quit_app");
});

for (const group of document.querySelectorAll(".segmented[data-setting]")) {
  group.addEventListener("click", (e) => {
    const b = e.target.closest("button[data-value]");
    if (!b) return;
    const key = group.dataset.setting;
    state.settings[key] = b.dataset.value;
    if (key === "dim") body.dataset.dim = b.dataset.value;
    renderSettings();
    if (key === "mapSize" || key === "lockSpot") renderMinimap();
    saveSettings({ [key]: b.dataset.value });
  });
}
for (const input of document.querySelectorAll("input[data-setting]")) {
  input.addEventListener("change", () => {
    const v = input.type === "checkbox" ? input.checked : input.value;
    state.settings[input.dataset.setting] = v;
    applyUiAttributes();
    saveSettings({ [input.dataset.setting]: v });
  });
}
function closeSelects(except) {
  for (const select of document.querySelectorAll(".select.open")) {
    if (select === except) continue;
    select.classList.remove("open");
    select.querySelector(".select-button").setAttribute("aria-expanded", "false");
  }
}

function openSelect(select) {
  closeSelects(select);
  const button = select.querySelector(".select-button");
  const menu = select.querySelector(".select-menu");
  const r = button.getBoundingClientRect();
  const room = innerHeight - r.bottom - 100;
  select.classList.toggle("up", room < menu.offsetHeight && r.top > menu.offsetHeight + 20);
  select.classList.add("open");
  button.setAttribute("aria-expanded", "true");
  const chosen = select.querySelector('.select-option[aria-selected="true"]') || select.querySelector(".select-option");
  chosen?.focus({ preventScroll: true });
}

function chooseOption(select, option) {
  const key = select.dataset.setting;
  closeSelects();
  select.querySelector(".select-button").focus({ preventScroll: true });
  if (state.settings[key] === option.dataset.value) return;
  state.settings[key] = option.dataset.value;
  applyUiAttributes();
  renderSettings();
  renderMinimap();
  saveSettings({ [key]: option.dataset.value });
}

for (const select of document.querySelectorAll(".select[data-setting]")) {
  const button = select.querySelector(".select-button");
  const menu = select.querySelector(".select-menu");
  button.addEventListener("click", () => (select.classList.contains("open") ? closeSelects() : openSelect(select)));
  button.addEventListener("keydown", (e) => {
    if (!["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    openSelect(select);
  });
  menu.addEventListener("click", (e) => {
    const option = e.target.closest(".select-option");
    if (option) chooseOption(select, option);
  });
  menu.addEventListener("keydown", (e) => {
    const options = [...menu.querySelectorAll(".select-option")];
    const index = options.indexOf(document.activeElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      options[(index + step + options.length) % options.length].focus();
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      options[e.key === "Home" ? 0 : options.length - 1].focus();
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (index >= 0) chooseOption(select, options[index]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeSelects();
      button.focus({ preventScroll: true });
    } else if (e.key === "Tab") {
      closeSelects();
    }
  });
}
document.addEventListener("mousedown", (e) => {
  if (!e.target.closest?.(".select")) closeSelects();
});

els.tabs.addEventListener("click", (e) => {
  const tab = e.target.closest(".tab");
  if (tab) setTab(tab.dataset.tab);
});
els.shortcutButton.addEventListener("click", () => (state.recording ? stopRecording() : startRecording()));

window.addEventListener("resize", () => {
  if (state.phase === "hidden") return;
  measure();
  renderMinimap();
  if (state.view === "picker" && layout() === "slider") renderRow(null);
});

listen("overlay://open", (e) => open(e.payload?.view || "picker"));
listen("overlay://close", () => close());

measure();
invoke("get_state")
  .then(applySnapshot)
  .catch(() => {})
  .finally(() => invoke("ready").catch(() => {}));
