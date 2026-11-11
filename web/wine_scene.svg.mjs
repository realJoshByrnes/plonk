// Plonk wine scene: one vector artwork for both the boot animation and the (settled) desktop wallpaper.
// sceneSVG(t) = full SVG string at time t; t >= DURATION is the settled state. Pure function of t (no globals mutated).
export const DURATION = 4.5;
export const VBW = 1600, VBH = 1000;
export const GX = 1150, SURF = 395, REST_Y = 393, TI = DURATION * 0.35; // glass centre x, wine surface y, cherry rest y, impact time
const SUN = [1390, 170], TABLE_Y = 770, REST_ROT = -8;

const f = (n) => (Math.round(n * 10) / 10).toString();
const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
const ss = (a, b, x) => { const t = clamp((x - a) / (b - a)); return t * t * (3 - 2 * t); };
const calm = (t) => 1 - ss(DURATION * 0.55, DURATION, t); // shimmer/drift amplitude, exactly 0 at DURATION
const settle = (t) => 1 - ss(DURATION * 0.8, DURATION, t);
function rng(seed) { let a = seed >>> 0; return () => { a = (a + 0x6d2b79f5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }

const BOWL_L = "M1018,300 C1006,410 1044,538 1150,556";
const BOWL = `${BOWL_L} C1256,538 1294,410 1282,300`;
const BOWL_CLOSED = `M1018,300 C1006,410 1044,538 1150,556 C1256,538 1294,410 1282,300 A132,24 0 0 1 1018,300 Z`;

// ---------- static pieces ----------
function bokeh() {
  const r = rng(7), cols = ["#ff3d8b", "#ff9a1f", "#ffd54a", "#18d6c4", "#ff5a2a", "#ffb0dc", "#3ee0ff"];
  let s = "";
  for (let i = 0; i < 46; i++) {
    const x = 380 + Math.pow(r(), 0.7) * 1250, y = 40 + r() * 720, rad = 14 + r() * 58;
    const side = clamp((x - 450) / 900); // calm on the left
    const o = (0.10 + r() * 0.30) * (0.25 + 0.75 * side);
    s += `<circle cx="${f(x)}" cy="${f(y)}" r="${f(rad)}" fill="${cols[(r() * cols.length) | 0]}" opacity="${f(o * 100) / 100}"/>`;
  }
  return s;
}

function glassStatic(id) {
  // everything of the glass except wine/cherry (back part and front part are split by caller)
  return id;
}

function defs() {
  const [sx, sy] = SUN;
  return `<defs>
<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a0a52"/><stop offset=".28" stop-color="#8d1470"/><stop offset=".5" stop-color="#e8366a"/><stop offset=".68" stop-color="#ff7a1c"/><stop offset=".78" stop-color="#ffc13a"/><stop offset="1" stop-color="#ffd978"/></linearGradient>
<linearGradient id="leftDark" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#12062e" stop-opacity=".78"/><stop offset=".35" stop-color="#1c0838" stop-opacity=".5"/><stop offset=".7" stop-color="#2a0a40" stop-opacity="0"/></linearGradient>
<linearGradient id="topDark" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#14052c" stop-opacity=".55"/><stop offset=".4" stop-color="#14052c" stop-opacity="0"/></linearGradient>
<radialGradient id="sunGlow" cx="${sx}" cy="${sy}" r="760" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff6d0" stop-opacity="1"/><stop offset=".07" stop-color="#ffe9a0" stop-opacity=".95"/><stop offset=".2" stop-color="#ffb23c" stop-opacity=".55"/><stop offset=".5" stop-color="#ff5a7a" stop-opacity=".22"/><stop offset="1" stop-color="#ff3d8b" stop-opacity="0"/></radialGradient>
<radialGradient id="rayg" cx="${sx}" cy="${sy}" r="1700" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff2c0" stop-opacity=".62"/><stop offset=".4" stop-color="#ffc870" stop-opacity=".28"/><stop offset="1" stop-color="#ff9a60" stop-opacity="0"/></radialGradient>
<linearGradient id="table" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#6b2a2e"/><stop offset=".12" stop-color="#3c1226"/><stop offset=".55" stop-color="#1c0a1c"/><stop offset="1" stop-color="#0f0612"/></linearGradient>
<linearGradient id="tableEdge" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ffc060" stop-opacity="0"/><stop offset=".6" stop-color="#ffd890" stop-opacity=".85"/><stop offset="1" stop-color="#fff0c0" stop-opacity=".95"/></linearGradient>
<radialGradient id="tableSheen" cx=".5" cy=".5" r=".5"><stop offset="0" stop-color="#ffb060" stop-opacity=".55"/><stop offset="1" stop-color="#ff7040" stop-opacity="0"/></radialGradient>
<linearGradient id="refl" x1="0" y1="${TABLE_Y + 30}" x2="0" y2="${TABLE_Y + 240}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff" stop-opacity=".5"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>
<mask id="reflMask" maskUnits="userSpaceOnUse" x="0" y="0" width="1600" height="1000"><rect x="900" y="${TABLE_Y + 30}" width="500" height="260" fill="url(#refl)"/></mask>
<radialGradient id="wine" cx=".5" cy=".6" r=".62"><stop offset="0" stop-color="#d11d45"/><stop offset=".45" stop-color="#9a0f33"/><stop offset=".85" stop-color="#5a0620"/><stop offset="1" stop-color="#2c0212"/></radialGradient>
<linearGradient id="wineSide" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#1a0108" stop-opacity=".55"/><stop offset=".22" stop-color="#1a0108" stop-opacity="0"/><stop offset=".7" stop-color="#ff3050" stop-opacity="0"/><stop offset=".92" stop-color="#ff5a70" stop-opacity=".28"/><stop offset="1" stop-color="#1a0108" stop-opacity=".5"/></linearGradient>
<radialGradient id="wineLit" cx=".5" cy=".5" r=".5"><stop offset="0" stop-color="#ff6a6a" stop-opacity=".75"/><stop offset=".5" stop-color="#ff2a50" stop-opacity=".3"/><stop offset="1" stop-color="#ff2a50" stop-opacity="0"/></radialGradient>
<radialGradient id="surfGrad" cx=".42" cy=".5" r=".6"><stop offset="0" stop-color="#e83a58"/><stop offset=".6" stop-color="#a8183c"/><stop offset="1" stop-color="#6a0a26"/></radialGradient>
<linearGradient id="glassFill" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#fff" stop-opacity=".34"/><stop offset=".12" stop-color="#ffd6e8" stop-opacity=".08"/><stop offset=".5" stop-color="#fff" stop-opacity=".02"/><stop offset=".85" stop-color="#bff6ff" stop-opacity=".1"/><stop offset="1" stop-color="#fff" stop-opacity=".38"/></linearGradient>
<linearGradient id="glassEdge" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#fff" stop-opacity=".85"/><stop offset=".3" stop-color="#fff" stop-opacity=".25"/><stop offset=".7" stop-color="#cff" stop-opacity=".3"/><stop offset="1" stop-color="#fff" stop-opacity=".9"/></linearGradient>
<linearGradient id="streak" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0"/><stop offset=".25" stop-color="#fff" stop-opacity=".95"/><stop offset=".75" stop-color="#fff" stop-opacity=".7"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>
<linearGradient id="stemG" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#fff" stop-opacity=".25"/><stop offset=".25" stop-color="#fff" stop-opacity=".9"/><stop offset=".45" stop-color="#d8e8ff" stop-opacity=".2"/><stop offset=".8" stop-color="#ffd0e0" stop-opacity=".25"/><stop offset="1" stop-color="#fff" stop-opacity=".7"/></linearGradient>
<radialGradient id="footG" cx=".35" cy=".4" r=".7"><stop offset="0" stop-color="#fff" stop-opacity=".55"/><stop offset=".6" stop-color="#ffd0e0" stop-opacity=".2"/><stop offset="1" stop-color="#fff" stop-opacity=".45"/></radialGradient>
<radialGradient id="caustic" cx=".5" cy=".5" r=".5"><stop offset="0" stop-color="#ffe0a0" stop-opacity=".95"/><stop offset=".25" stop-color="#ff4a60" stop-opacity=".7"/><stop offset=".7" stop-color="#c0104a" stop-opacity=".25"/><stop offset="1" stop-color="#c0104a" stop-opacity="0"/></radialGradient>
<radialGradient id="cherryG" cx=".35" cy=".3" r=".8"><stop offset="0" stop-color="#ff6a74"/><stop offset=".35" stop-color="#e01834"/><stop offset=".8" stop-color="#8a0720"/><stop offset="1" stop-color="#4a0212"/></radialGradient>
<radialGradient id="flash" cx=".5" cy=".5" r=".5"><stop offset="0" stop-color="#fff" stop-opacity="1"/><stop offset=".4" stop-color="#ffd0d8" stop-opacity=".5"/><stop offset="1" stop-color="#ff90a0" stop-opacity="0"/></radialGradient>
<radialGradient id="dropG" cx=".35" cy=".3" r=".8"><stop offset="0" stop-color="#ff8a90"/><stop offset=".5" stop-color="#d01838"/><stop offset="1" stop-color="#7a0824"/></radialGradient>
<linearGradient id="tailG" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ff3050" stop-opacity="0"/><stop offset="1" stop-color="#ff3050" stop-opacity=".55"/></linearGradient>
<radialGradient id="vig" cx=".55" cy=".45" r=".8"><stop offset=".6" stop-color="#0a0218" stop-opacity="0"/><stop offset="1" stop-color="#0a0218" stop-opacity=".5"/></radialGradient>
<clipPath id="bowlClip"><path d="${BOWL_CLOSED}"/></clipPath>
<clipPath id="wineClip"><path d="${BOWL_CLOSED}"/><rect x="0" y="0" width="0" height="0"/></clipPath>
<clipPath id="underClip"><rect x="900" y="${SURF}" width="500" height="300"/></clipPath>
<clipPath id="belowClip"><rect x="900" y="${SURF + 5}" width="500" height="300"/></clipPath>
<clipPath id="wineBody"><rect x="900" y="${SURF}" width="500" height="300"/></clipPath>
<clipPath id="surfClip"><ellipse cx="${GX}" cy="${SURF}" rx="129" ry="21"/></clipPath>
<filter id="bl8" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="8"/></filter>
<filter id="bl3" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="3"/></filter>
<filter id="bl18" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="18"/></filter>
</defs>`;
}

function background() {
  const [sx, sy] = SUN;
  return `<rect width="1600" height="1000" fill="url(#sky)"/>
<g filter="url(#bl3)" style="mix-blend-mode:screen">${bokeh()}</g>
<rect width="1600" height="1000" fill="url(#sunGlow)" style="mix-blend-mode:screen"/>
<circle cx="${sx}" cy="${sy}" r="52" fill="#fffbe8"/><circle cx="${sx}" cy="${sy}" r="90" fill="#fff0b0" opacity=".45" filter="url(#bl18)"/>
<rect width="1600" height="1000" fill="url(#leftDark)"/><rect width="1600" height="1000" fill="url(#topDark)"/>`;
}

function table() {
  const y = TABLE_Y;
  return `<rect x="0" y="${y}" width="1600" height="${1000 - y}" fill="url(#table)"/>
<ellipse cx="1390" cy="${y + 90}" rx="190" ry="95" fill="url(#tableSheen)" style="mix-blend-mode:screen"/>
<ellipse cx="1390" cy="${y + 130}" rx="50" ry="130" fill="#ffd890" opacity=".28" filter="url(#bl18)"/>
<rect x="0" y="${y - 1}" width="1600" height="3" fill="url(#tableEdge)"/>
<rect x="0" y="${y + 2}" width="1600" height="26" fill="#ffb060" opacity=".14" filter="url(#bl8)"/>`;
}

// glass body in 3 parts: glassBack (behind wine), glassFront (over wine+cherry), plus reusable shape for reflection
function glassBack() {
  return `<ellipse cx="${GX}" cy="300" rx="132" ry="24" fill="#2a0a30" opacity=".25"/>
<path d="${BOWL_CLOSED}" fill="url(#glassFill)"/>
<ellipse cx="${GX}" cy="300" rx="132" ry="24" fill="none" stroke="#fff" stroke-opacity=".45" stroke-width="2"/>`;
}
function wineStatic() {
  return `<g clip-path="url(#bowlClip)"><g clip-path="url(#wineBody)">
<rect x="1000" y="${SURF - 30}" width="300" height="200" fill="url(#wine)"/>
<rect x="1000" y="${SURF - 30}" width="300" height="200" fill="url(#wineSide)"/>
<ellipse cx="1120" cy="505" rx="62" ry="38" fill="url(#wineLit)" style="mix-blend-mode:screen" transform="rotate(-24 1120 505)"/>
<ellipse cx="1150" cy="548" rx="40" ry="10" fill="#ff3a50" opacity=".45" filter="url(#bl8)"/>
<ellipse cx="1150" cy="${SURF + 21}" rx="129" ry="14" fill="#14020a" opacity=".5" filter="url(#bl8)"/>
</g></g>`;
}
function glassFront() {
  return `<g clip-path="url(#bowlClip)">
<path d="M1075,318 C1052,380 1058,452 1082,505" stroke="url(#streak)" stroke-width="10" fill="none" stroke-linecap="round" opacity=".9"/>
<path d="M1060,322 C1034,390 1044,470 1076,520" stroke="#fff" stroke-opacity=".28" stroke-width="3" fill="none" stroke-linecap="round"/>
<path d="M1255,330 C1272,392 1262,452 1236,500" stroke="url(#streak)" stroke-width="6" fill="none" stroke-linecap="round" opacity=".75"/>
<path d="M1230,520 C1196,548 1150,552 1120,546" stroke="#ffd0b0" stroke-opacity=".55" stroke-width="3" fill="none" stroke-linecap="round"/>
<g transform="rotate(-9 1105 380)" opacity=".2" style="mix-blend-mode:screen"><rect x="1088" y="338" width="32" height="46" rx="3" fill="#fff"/><rect x="1124" y="338" width="32" height="46" rx="3" fill="#fff"/><rect x="1088" y="388" width="32" height="46" rx="3" fill="#fff"/><rect x="1124" y="388" width="32" height="46" rx="3" fill="#fff"/></g>
<ellipse cx="1218" cy="352" rx="22" ry="38" fill="#ffe0a0" opacity=".28" filter="url(#bl8)"/>
</g>
<path d="${BOWL}" fill="none" stroke="url(#glassEdge)" stroke-width="2.6"/>
<path d="M1018,300 A132,24 0 0 0 1282,300" fill="none" stroke="#fff" stroke-opacity=".9" stroke-width="2.4"/>
<path d="M1060,322 A132,24 0 0 0 1130,324" fill="none" stroke="#fff" stroke-width="4" stroke-linecap="round" opacity=".9"/>
<g style="mix-blend-mode:screen"><circle cx="1282" cy="302" r="4" fill="#fff"/><path d="M1282,284V320M1264,302H1300" stroke="#fff" stroke-width="1.6" opacity=".9"/><circle cx="1018" cy="300" r="3" fill="#fff"/><path d="M1018,288V312M1006,300H1030" stroke="#fff" stroke-width="1.2" opacity=".8"/></g>
<path d="M1141,549 C1146,600 1146,700 1141,772 L1159,772 C1154,700 1154,600 1159,549 Z" fill="url(#stemG)"/>
<path d="M1144,560 C1146,620 1146,700 1144,768" stroke="#fff" stroke-width="2" opacity=".9" fill="none"/>
<path d="M1141,770 C1138,788 1090,788 1048,799 L1252,799 C1210,788 1162,788 1159,770 Z" fill="url(#footG)"/>
<ellipse cx="${GX}" cy="799" rx="104" ry="16" fill="url(#footG)"/>
<ellipse cx="${GX}" cy="799" rx="104" ry="16" fill="none" stroke="#fff" stroke-opacity=".75" stroke-width="2"/>
<ellipse cx="${GX}" cy="793" rx="62" ry="9" fill="none" stroke="#fff" stroke-opacity=".4" stroke-width="2"/>
<path d="M1064,806 A104,16 0 0 0 1130,814" stroke="#fff" stroke-width="3.5" fill="none" stroke-linecap="round" opacity=".85"/>
<ellipse cx="${GX}" cy="552" rx="26" ry="9" fill="#fff" opacity=".18" filter="url(#bl3)"/>`;
}
function reflection() {
  return `<g mask="url(#reflMask)" opacity=".85"><g transform="translate(0 ${2 * 800 - 6}) scale(1 -1)">
${glassBack()}${wineStatic()}<ellipse cx="${GX}" cy="${SURF}" rx="129" ry="21" fill="url(#surfGrad)"/>${glassFront()}</g></g>`;
}
function shadowsCaustic() {
  return `<ellipse cx="1060" cy="822" rx="210" ry="30" fill="#0a0210" opacity=".6" filter="url(#bl18)"/>
<ellipse cx="${GX}" cy="806" rx="112" ry="14" fill="#05010a" opacity=".7" filter="url(#bl8)"/>
<g style="mix-blend-mode:screen"><ellipse cx="930" cy="858" rx="150" ry="36" fill="url(#caustic)" transform="rotate(-6 930 858)"/>
<ellipse cx="900" cy="854" rx="46" ry="9" fill="#ffe8b0" opacity=".85" filter="url(#bl3)" transform="rotate(-6 900 854)"/>
<ellipse cx="1010" cy="850" rx="70" ry="12" fill="#ff7a90" opacity=".5" filter="url(#bl8)"/></g>`;
}

// ---------- animated elements: every one is a <g> animated only via transform/opacity (WAAPI keyframes sampled from the physics) ----------
// spec: { id, layer, inner, t0, t1, dt, state(t) -> { tr?, o } }
const GRAV = 1500;
const px = (x, y) => `translate(${f(x)}px,${f(y)}px)`;
const fl = (n) => Math.round(n * 1000) / 1000;

function cherryPose(t) {
  const st = settle(t);
  if (t < TI) {
    const a = 2 * ((SURF - 30) + 60) / (TI * TI), y = -60 + 0.5 * a * t * t, v = a * t;
    return { x: GX, y, rot: REST_ROT + 24 * (1 - t / TI), v, fall: true };
  }
  const tau = t - TI, vi = 2 * ((SURF - 30) + 60) / TI, c0 = (SURF - 30) - REST_Y, k = 2.4, w = 9;
  const s0 = (vi + k * c0) / w;
  const y = REST_Y + st * Math.exp(-k * tau) * (c0 * Math.cos(w * tau) + s0 * Math.sin(w * tau));
  return { x: GX, y, rot: REST_ROT + 14 * st * Math.exp(-2 * tau) * Math.sin(7 * tau), v: 0, fall: false };
}
const CHERRY_BODY = "M0,-26 C12,-38 36,-26 33,2 C31,28 10,36 0,34 C-10,36 -31,28 -33,2 C-36,-26 -12,-38 0,-26 Z";
const CHERRY_INNER = `<path d="M0,-28 C6,-52 12,-66 30,-80" stroke="#4e5a1c" stroke-width="4.4" fill="none" stroke-linecap="round"/>
<path d="M-1,-28 C5,-52 11,-66 29,-80" stroke="#a6b44a" stroke-width="1.4" fill="none" stroke-linecap="round"/>
<path d="${CHERRY_BODY}" fill="url(#cherryG)"/>
<path d="M0,-26 C-4,-18 -3,-8 0,-2" stroke="#4a0212" stroke-width="3" fill="none" opacity=".5" stroke-linecap="round"/>
<ellipse cx="-13" cy="-9" rx="9" ry="14" fill="#fff" opacity=".85" transform="rotate(28 -13 -9)"/>
<ellipse cx="-14" cy="-13" rx="3.4" ry="5.5" fill="#fff" transform="rotate(28 -14 -13)"/>
<path d="M18,22 C10,29 -2,31 -10,28" stroke="#ff8a8a" stroke-width="3" fill="none" opacity=".55" stroke-linecap="round"/>`;
const cherryTr = (p) => `${px(p.x, p.y)} rotate(${f(p.rot)}deg) scale(1.3)`;

let _specs;
function specs() {
  if (_specs) return _specs;
  const S = [], D = DURATION;
  // rays
  { const r = rng(11), [sx, sy] = SUN, L = 2000;
    for (let i = 0; i < 15; i++) {
      const a = (100 + i * 4.4 + (r() - 0.5) * 2) * Math.PI / 180, w = (0.8 + r() * 2.1) * Math.PI / 180, base = 0.35 + r() * 0.65, fr = 1.3 + r();
      S.push({ id: "ray" + i, layer: "gRays", t0: 0, t1: D, dt: 0.1,
        inner: `<polygon points="${sx},${sy} ${f(sx + L * Math.cos(a - w))},${f(sy + L * Math.sin(a - w))} ${f(sx + L * Math.cos(a + w))},${f(sy + L * Math.sin(a + w))}" fill="url(#rayg)"/>`,
        state: (t) => ({ o: fl(base * (1 + 0.4 * Math.sin(t * fr + i * 1.7) * calm(t)) * 0.5) }) });
    } }
  // sparkles + dust
  { const r = rng(23);
    for (let i = 0; i < 30; i++) {
      const x = 760 + r() * 800, y = 60 + r() * 740, sz = 2 + r() * 5, ph = r() * 6.28, big = r() < 0.3, fr = 1.5 + r() * 2, ob = 0.35 + r() * 0.6;
      const inner = big ? `<g transform="translate(${f(x)} ${f(y)})"><path d="M0,${f(-sz * 3.2)}L${f(sz * .5)},0L0,${f(sz * 3.2)}L${f(-sz * .5)},0Z M${f(-sz * 3.2)},0L0,${f(sz * .5)}L${f(sz * 3.2)},0L0,${f(-sz * .5)}Z" fill="#fff"/></g>`
        : `<circle cx="${f(x)}" cy="${f(y)}" r="${f(sz * .5)}" fill="#fff4d0"/>`;
      S.push({ id: "sp" + i, layer: "gSpark", inner, t0: 0, t1: D, dt: 0.1, state: (t) => {
        const c = calm(t), tw = 0.75 + 0.25 * Math.sin(t * fr + ph) * c;
        return { tr: px(Math.sin(t * 0.6 + ph) * 9 * c, Math.cos(t * 0.5 + ph) * 7 * c), o: fl(ob * tw * (0.4 + 0.6 * clamp((x - 760) / 500))) };
      } });
    } }
  // wobble of the wine surface (the persistent surface group)
  S.push({ id: "wob", layer: "wob", static: true, t0: TI, t1: D, dt: 1 / 30, state: (t) => {
    const tau = t - TI, a = tau > 0 ? 2.4 * settle(t) * Math.exp(-1.8 * tau) * Math.sin(10 * tau) : 0;
    return { tr: `${px(GX, SURF)} rotate(${fl(a)}deg) ${px(-GX, -SURF)}`, o: 1 };
  } });
  // ripples
  for (let k = 0; k < 5; k++) {
    const t0 = TI + k * 0.28;
    S.push({ id: "rip" + k, layer: "gRipple", t0, t1: t0 + 1.6, dt: 1 / 30,
      inner: `<ellipse cx="0" cy="0" rx="100" ry="16.5" fill="none" stroke="#ffd4dc" stroke-width="2.2" vector-effect="non-scaling-stroke"/><ellipse cx="0" cy="2" rx="100" ry="16.5" fill="none" stroke="#3a0212" stroke-opacity=".5" stroke-width="2" vector-effect="non-scaling-stroke"/>`,
      state: (t) => { const s0 = (t - t0) / 1.6; if (s0 <= 0 || s0 >= 1) return { tr: `${px(GX, SURF + 1)} scale(.2)`, o: 0 };
        const e = 1 - Math.pow(1 - s0, 2); return { tr: `${px(GX, SURF + 1)} scale(${fl((22 + 110 * e) / 100)})`, o: fl(0.8 * Math.pow(1 - s0, 1.4)) }; } });
  }
  // bubbles
  { const r = rng(5);
    for (let i = 0; i < 14; i++) {
      const t0 = TI + 0.12 + r() * 0.9, life = 0.9 + r() * 0.7, bx = GX + (r() - 0.5) * 70, rad = 1.6 + r() * 3.2, sw = r() * 6.28;
      S.push({ id: "bub" + i, layer: "gBub", t0, t1: t0 + life, dt: 1 / 20,
        inner: `<circle r="${f(rad)}" fill="#ff7a8a" fill-opacity=".25" stroke="#ffd0d8" stroke-width="1"/><circle cx="${f(-rad * .3)}" cy="${f(-rad * .3)}" r="${f(rad * .35)}" fill="#fff"/>`,
        state: (t) => { const u = clamp((t - t0) / life);
          return { tr: px(bx + Math.sin(u * 9 + sw) * 5, 450 + (SURF + 6 - 450) * u + 20 * (1 - u) * (i % 3)), o: u <= 0 || u >= 1 ? 0 : fl(Math.min(1, u * 6) * (1 - ss(0.85, 1, u))) }; } });
    } }
  // cherry, submerged tint, meniscus ring
  S.push({ id: "cherry", layer: "gCherry", inner: CHERRY_INNER, t0: 0, t1: D, dt: 1 / 60, state: (t) => ({ tr: cherryTr(cherryPose(t)), o: 1 }) });
  S.push({ id: "tint", layer: "gTint", inner: `<path d="${CHERRY_BODY}" fill="#7a0a26" opacity=".58"/>`, t0: 0, t1: D, dt: 1 / 60, state: (t) => ({ tr: cherryTr(cherryPose(t)), o: 1 }) });
  S.push({ id: "ring", layer: "gRing", t0: 0, t1: D, dt: 1 / 30,
    inner: `<path d="M${GX - 36},${SURF + 3} A36,6.5 0 0 0 ${GX + 36},${SURF + 3}" fill="none" stroke="#ffc8d0" stroke-opacity=".8" stroke-width="1.8"/><path d="M${GX - 44},${SURF + 5} A44,8 0 0 0 ${GX + 44},${SURF + 5}" fill="none" stroke="#ff8a98" stroke-opacity=".35" stroke-width="1.4"/>`,
    state: (t) => ({ o: fl(1 - ss(28, 46, Math.abs(cherryPose(t).y - SURF))) }) });
  // motion-blur streak above the falling cherry
  S.push({ id: "streak", layer: "gFx", t0: 0, t1: TI + 0.05, dt: 1 / 60,
    inner: `<path d="M-27,0 L27,0 L18,-150 L-18,-150 Z" fill="url(#tailG)"/>`,
    state: (t) => { const p = cherryPose(t), len = Math.min(150, p.v * 0.2); return { tr: `${px(p.x, p.y)} scale(1,${fl(Math.max(0.01, len / 150))})`, o: p.fall && p.v > 200 ? 1 : 0 }; } });
  // crown spikes + jet + flash
  { const r = rng(3), cT = 0.42;
    for (let i = 0; i < 16; i++) {
      const a = (i / 16) * Math.PI * 2 + r() * 0.2, hh = 40 + r() * 55, lean = Math.cos(a) * 26, w = 5 + r() * 3;
      S.push({ id: "cr" + i, layer: "gFx", t0: TI, t1: TI + cT, dt: 1 / 40,
        inner: `<path d="M${f(-w)},0 Q${f(lean * .3 - w * .5)},${f(-hh * .6)} ${f(lean)},${f(-hh)} Q${f(lean * .3 + w * .5)},${f(-hh * .6)} ${f(w)},0 Z" fill="#c4143a"/><path d="M${f(-w * .3)},-2 Q${f(lean * .3)},${f(-hh * .6)} ${f(lean * .95)},${f(-hh * .95)}" stroke="#ffc0c8" stroke-width="1.4" fill="none" opacity=".7"/>`,
        state: (t) => { const u = clamp((t - TI) / cT), h = Math.sin(Math.PI * Math.pow(u, 0.7)), rr = 30 + 46 * u;
          return { tr: `${px(GX + Math.cos(a) * rr, SURF + Math.sin(a) * rr * 0.17)} scale(${fl(Math.max(0.01, h))})`, o: u <= 0 || u >= 1 ? 0 : fl(1 - u * u * 0.5) }; } });
    }
    S.push({ id: "jet", layer: "gFx", t0: TI, t1: TI + cT, dt: 1 / 40,
      inner: `<path d="M-15,2 Q-11,-72 0,-120 Q11,-72 15,2 Z" fill="#b01034"/><circle cx="0" cy="-124" r="6" fill="#e02848"/>`,
      state: (t) => { const u = clamp((t - TI) / cT), h = Math.sin(Math.PI * Math.pow(u, 0.6));
        return { tr: `${px(GX, SURF)} scale(1,${fl(Math.max(0.01, h))})`, o: u <= 0 || u >= 1 ? 0 : fl(1 - u * u * 0.5) }; } });
    S.push({ id: "flash", layer: "gFx", t0: TI, t1: TI + 0.22, dt: 1 / 60,
      inner: `<g style="mix-blend-mode:screen"><circle cx="0" cy="0" r="170" fill="url(#flash)"/><path d="M0,-110V56M-170,0H170" stroke="#fff" stroke-width="2" opacity=".7"/></g>`,
      state: (t) => { const u = clamp((t - TI) / 0.22); return { tr: `${px(GX, SURF - 6)} scale(${fl((40 + 130 * u) / 170)})`, o: u <= 0 || u >= 1 ? 0 : fl((1 - u) * (1 - u)) }; } });
  }
  // droplets (ballistic) and table splats
  { const r = rng(99);
    for (let i = 0; i < 34; i++) {
      const big = i < 6, spd = 240 + r() * 480 + (big ? 120 : 0), dir = (r() - 0.5) * 2 * 1.15;
      const vx = Math.sin(dir) * spd * 0.9, vy0 = -Math.cos(dir) * spd * 1.15, rad = (big ? 4.5 + r() * 2.5 : 1.8 + r() * 3) * 1.3;
      const x0 = GX + (r() - 0.5) * 70, y0 = SURF - 2, d = r() * 0.06, ty = 828 + r() * 70;
      let qEnd = 3, land = false;
      for (let q = 0.004; q < 3; q += 0.004) {
        const x = x0 + vx * q, y = y0 + vy0 * q + 0.5 * GRAV * q * q, vy = vy0 + GRAV * q;
        if (vy > 0 && y >= SURF && Math.abs(x - GX) < 130) { qEnd = q; break; }
        if (y >= ty) { qEnd = q; land = true; break; }
      }
      const t0 = TI + d, t1 = t0 + qEnd + 0.04;
      S.push({ id: "dr" + i, layer: "gFx", t0, t1, dt: 1 / 60,
        inner: `<ellipse rx="${f(rad * 1.45)}" ry="${f(rad)}" fill="url(#dropG)"/><ellipse cx="${f(-rad * .3)}" cy="${f(-rad * .35)}" rx="${f(rad * .35)}" ry="${f(rad * .22)}" fill="#fff" opacity=".9"/>`,
        state: (t) => { const q = Math.min(t - t0, qEnd), x = x0 + vx * q, y = y0 + vy0 * q + 0.5 * GRAV * q * q, vy = vy0 + GRAV * q;
          return { tr: `${px(x, y)} rotate(${fl(Math.atan2(vy, vx) * 180 / Math.PI)}deg)`, o: t <= t0 ? 0 : t - t0 >= qEnd ? fl(1 - (t - t0 - qEnd) / 0.04) : 1 }; } });
      if (land) {
        const lx = x0 + vx * qEnd, s0 = t0 + qEnd;
        S.push({ id: "sp_" + i, layer: "gFx", t0: s0, t1: s0 + 0.9, dt: 1 / 20,
          inner: `<ellipse rx="${f(rad)}" ry="${f(rad * .5)}" fill="#c01238"/><ellipse cx="-1" cy="-1" rx="${f(rad * .5)}" ry="${f(rad * .2)}" fill="#fff" opacity=".7"/>`,
          state: (t) => { const u = clamp((t - s0) / 0.9); return { tr: `${px(lx, ty)} scale(${fl(1.6 + u * 2.4)},${fl(1 + u * 1.2)})`, o: t <= s0 ? 0 : fl(0.8 * (1 - u)) }; } });
      }
    } }
  return (_specs = S);
}

const styleOf = (s, t) => { const st = s.state(t); return `transform-origin:0 0;${st.tr ? `transform:${st.tr};` : ""}opacity:${st.o}`; };
const markupOf = (s, t) => `<g id="${s.id}" style="${styleOf(s, t)}">${s.inner}</g>`;
const layerMarkup = (layer, t, onlyVisible) => specs().filter((s) => s.layer === layer && !s.static && (!onlyVisible || s.state(t).o > 0.001)).map((s) => markupOf(s, t)).join("");

function surfaceStatic() {
  return `<ellipse cx="${GX}" cy="${SURF}" rx="129" ry="21" fill="url(#surfGrad)"/>
<ellipse cx="${GX - 38}" cy="${SURF - 3}" rx="60" ry="9" fill="#ff8a98" opacity=".28" filter="url(#bl3)"/>
<ellipse cx="${GX + 62}" cy="${SURF + 3}" rx="30" ry="5" fill="#ffd0b0" opacity=".25" filter="url(#bl3)"/>`;
}
const meniscus = () => `<path d="M${GX - 129},${SURF} A129,21 0 0 0 ${GX + 129},${SURF}" fill="none" stroke="#ffc0c8" stroke-opacity=".75" stroke-width="2"/><path d="M${GX - 129},${SURF - 2} A129,21 0 0 0 ${GX + 129},${SURF - 2}" fill="none" stroke="#fff" stroke-opacity=".3" stroke-width="5" filter="url(#bl3)"/>`;

// Scene 0: the pre-splash frame, self-contained (inline it in the page). Contains empty layers the intro fills.
export function scene0SVG(o = {}) {
  const D = DURATION, size = o.w ? ` width="${o.w}" height="${o.h}"` : "", style = o.style ? ` style="${o.style}"` : "", id = o.id ? ` id="${o.id}"` : "";
  const wobS = specs().find((s) => s.id === "wob");
  return `<svg${id} xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1600 1000" preserveAspectRatio="xMaxYMid slice"${size}${style}>${defs()}
<g id="bg">${background()}</g>
<g id="gRays" style="mix-blend-mode:screen">${layerMarkup("gRays", D)}</g>
<g id="table">${table()}${shadowsCaustic()}${reflection()}</g>
<g id="glass">${glassBack()}${wineStatic()}</g>
<g id="wob" style="${styleOf(wobS, D)}"><g clip-path="url(#bowlClip)">${surfaceStatic()}<g id="gRipple" clip-path="url(#surfClip)"></g><g id="gBub" clip-path="url(#underClip)"></g></g></g>
<g id="gCherry"></g><g clip-path="url(#belowClip)"><g id="gTint"></g></g><g id="gRing"></g>
${meniscus()}
<g id="glassFront">${glassFront()}</g>
<g id="gFx"></g>
<g id="gSpark" style="mix-blend-mode:screen">${layerMarkup("gSpark", D)}</g>
<rect width="1600" height="1000" fill="url(#vig)"/>
</svg>`;
}

// Settled scene = scene 0 + the persistent animated elements at their end state (the wallpaper).
export function sceneSVG(o = {}) {
  const D = DURATION;
  let s = scene0SVG(o);
  for (const layer of ["gCherry", "gTint", "gRing"]) s = s.replace(`<g id="${layer}"></g>`, `<g id="${layer}">${layerMarkup(layer, D, true)}</g>`);
  return s;
}

// Fill the transient layers of a mounted scene-0 <svg> and return the list of WAAPI animations (not yet started effects are created paused=false).
export function startAnimations(svg, { paused = false } = {}) {
  const T = (id) => svg.querySelector("#" + id);
  for (const layer of ["gRipple", "gBub", "gCherry", "gTint", "gRing", "gFx"]) {
    const g = T(layer); if (g) g.insertAdjacentHTML("beforeend", layerMarkup(layer, 0, false));
  }
  const anims = [];
  for (const s of specs()) {
    const el = T(s.id); if (!el) continue;
    const n = Math.max(1, Math.ceil((s.t1 - s.t0) / s.dt)), frames = [];
    for (let i = 0; i <= n; i++) { const st = s.state(s.t0 + (s.t1 - s.t0) * i / n), k = { offset: i / n, opacity: st.o }; if (st.tr) k.transform = st.tr; frames.push(k); }
    const a = el.animate(frames, { delay: Math.round(s.t0 * 1000), duration: Math.max(1, Math.round((s.t1 - s.t0) * 1000)), fill: "both", easing: "linear" });
    if (paused) a.pause();
    anims.push(a);
  }
  return anims;
}
export const animatedIds = () => specs().map((s) => s.id);
