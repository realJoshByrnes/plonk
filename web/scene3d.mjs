// Plonk boot scene: a glass of red wine on a polished walnut counter, side-lit by a golden-hour window;
// a cherry drops in with a splash. One full-screen triangle, one big fragment shader (analytic + sphere-traced),
// resolution independent: rendered live at whatever size the desktop is. t (seconds) fully determines the frame.
export const DURATION = 4.5;
export const T_IMPACT = 1.6;

const VS = `#version 300 es
void main(){ vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)); gl_Position = vec4(p * 2. - 1., 0., 1.); }`;

const FS = `#version 300 es
precision highp float;
precision highp int;
uniform vec2 uRes;
uniform float uT;
uniform float uPre;
uniform float uTI;
uniform float uDur;
uniform float uHdr;
out vec4 fragColor;

#define PI 3.14159265
const vec3 SUN = vec3(-0.7134, 0.6143, 0.3369);
const vec3 SUNC = vec3(1.0, 0.66, 0.34);
const float WX = -3.6;
const float Y0 = 0.79;
const float CR = 0.085;
const vec3 BC = vec3(0., 0.77, 0.);
const vec3 BR = vec3(0.31, 0.40, 0.31);
const float WALL = 0.016;
const float ZB = -1.3;
const float TANV = 0.29;
const vec3 CAM = vec3(0.15, 1.55, 3.7);
const vec3 SIGW = vec3(3.0, 11., 8.5);
const vec3 SIGG = vec3(0.9, 0.35, 0.6);

// ---------------------------------------------------------------- noise
float h11(float p){ p = fract(p * .1031); p *= p + 33.33; p *= p + p; return fract(p); }
float h21(vec2 p){ vec3 p3 = fract(vec3(p.xyx) * .1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
vec2 h22(vec2 p){ vec3 p3 = fract(vec3(p.xyx) * vec3(.1031, .1030, .0973)); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.xx + p3.yz) * p3.zy); }
float h31(vec3 p3){ p3 = fract(p3 * .1031); p3 += dot(p3, p3.zyx + 31.32); return fract((p3.x + p3.y) * p3.z); }
float vn(vec2 p){ vec2 i = floor(p), f = fract(p); f = f * f * (3. - 2. * f);
  return mix(mix(h21(i), h21(i + vec2(1, 0)), f.x), mix(h21(i + vec2(0, 1)), h21(i + vec2(1, 1)), f.x), f.y); }
float vn3(vec3 p){ vec3 i = floor(p), f = fract(p); f = f * f * (3. - 2. * f);
  return mix(mix(mix(h31(i), h31(i + vec3(1, 0, 0)), f.x), mix(h31(i + vec3(0, 1, 0)), h31(i + vec3(1, 1, 0)), f.x), f.y),
             mix(mix(h31(i + vec3(0, 0, 1)), h31(i + vec3(1, 0, 1)), f.x), mix(h31(i + vec3(0, 1, 1)), h31(i + vec3(1, 1, 1)), f.x), f.y), f.z); }
float smin(float a, float b, float k){ float h = max(k - abs(a - b), 0.) / k; return min(a, b) - h * h * k * .25; }
float smax(float a, float b, float k){ return -smin(-a, -b, k); }

// ---------------------------------------------------------------- window, sun, art direction
float winMask(float wy, float wz, float bl){
  float cz = 1.5;
  float fz = abs(fract(wz / cz + .5) - .5) * cz;
  float mz = smoothstep(.27, .27 + bl, fz);
  float my = smoothstep(.08, .08 + bl, abs(wy - 3.7));
  float ext = smoothstep(1., 1. + bl, wy) * smoothstep(6.4, 6.4 - bl, wy) * smoothstep(-1., -1. + bl * 4., wz);
  float gain = .65 + .35 * h21(vec2(floor(wz / cz + .5), floor(wy / 3.7)));
  return mz * my * ext * gain;
}
float sunMask(vec3 p){
  float s = (p.x - WX) / (-SUN.x);
  return winMask(p.y + SUN.y * s, p.z + SUN.z * s, .008 + .005 * s);
}
float art(float x){ return mix(.14, 1., smoothstep(-1.7, .8, x)); }

// ---------------------------------------------------------------- environment (dim bar, bokeh, window)
vec3 bokehLayer(vec2 w, float cs, float seed, float dens, float bl, float warmOnly, float yMask){
  vec2 id = floor(w / cs); vec3 acc = vec3(0.);
  for(int j = -1; j <= 1; j++) for(int i = -1; i <= 1; i++){
    vec2 cid = id + vec2(float(i), float(j));
    if(h21(cid + seed) > dens) continue;
    vec2 jt = h22(cid * 1.7 + seed) - .5;
    vec2 cc = (cid + .5 + jt * .6) * cs;
    float R = cs * (.28 + .2 * h21(cid + seed + 9.));
    float d = length(w - cc);
    float k = h21(cid + seed + 4.);
    vec3 col = k < .45 ? vec3(1., .5, .12) : k < .62 ? vec3(.95, .12, .45) : k < .82 ? vec3(.08, .65, .7) : vec3(1., .82, .55);
    if(warmOnly > .5) col = k < .5 ? vec3(1., .55, .15) : vec3(1., .8, .5);
    float soft = R * (.06 + .35 * bl);
    float disk = 1. - smoothstep(R - soft, R + soft, d);
    float ring = .75 + .6 * smoothstep(R * .6, R, d);
    acc += col * disk * ring * (.5 + 1.4 * h21(cid + seed + 2.));
  }
  return acc * yMask;
}
vec3 barWall(vec2 w, float bl){
  float dimL = mix(.06, 1., smoothstep(-3.6, 1.4, w.x));
  vec3 c = vec3(.05, .024, .016) * (.5 + .6 * smoothstep(-1.2, 1.2, w.y));
  c += vec3(.5, .22, .06) * .34 * exp(-pow((w.y - .5) / .7, 2.));
  c += vec3(.03, .2, .26) * .3 * exp(-pow((w.y - 1.5) / .8, 2.));
  c += vec3(.35, .05, .2) * .2 * exp(-pow((w.y + .3) / .5, 2.)) * smoothstep(-2., 1., w.x);
  // bottles on a shelf
  float cw = .55, ci = floor(w.x / cw), fx = (w.x / cw - ci - .5) * cw;
  float yy = w.y + .55;
  float soft = .05 + .12 * bl;
  float width = mix(.25, .085, smoothstep(.55, .85, yy));
  float top = 1.05 + .4 * h11(ci);
  float sil = smoothstep(width + soft, width - soft, abs(fx)) * smoothstep(top + soft, top - soft, yy) * smoothstep(-soft, soft, yy);
  float bk = h11(ci * 5.3);
  vec3 bcol = bk < .4 ? vec3(.02, .07, .035) : bk < .75 ? vec3(.16, .06, .015) : vec3(.12, .015, .02);
  c = mix(c, bcol * (.5 + 1.6 * exp(-pow((w.y - .5) / .8, 2.))), sil * .85);
  c += vec3(1., .6, .3) * .5 * exp(-pow((w.y + .55) / .03, 2.)) * smoothstep(-4., 0., w.x);
  c += bokehLayer(w - vec2(0., 0.), .78, 3., .3, bl, 0., smoothstep(-.9, -.3, w.y) * smoothstep(2.2, 1.5, w.y)) * .65;
  float cur = .95 + .3 * cos(w.x * .75);
  c += bokehLayer(w, .36, 11., .55, bl, 1., exp(-pow((w.y - cur) / .35, 2.))) * 1.4;
  c += vec3(1., .72, .42) * 3.2 * smoothstep(4.4, 4.7, w.y) * smoothstep(5.4, 5.0, w.y) * smoothstep(-2., 0., w.x) * smoothstep(7., 5., w.x);
  c += vec3(.55, .3, .16) * .5 * smoothstep(2.5, 4.5, w.y) * smoothstep(-3., 1., w.x);
  return c * dimL;
}
vec3 env(vec3 rd, float bl){
  vec3 c = mix(vec3(.012, .016, .026), vec3(.03, .02, .02), smoothstep(-.2, .5, rd.y));
  if(rd.z < -.02){ float s = 9.7 / (-rd.z); c += barWall(vec2(rd.x * s, 1.55 + rd.y * s), bl); }
  else c += vec3(.05, .03, .02) * smoothstep(-.3, .6, rd.y) * .6;
  if(rd.x < -.12){
    float s = WX / rd.x;
    float wy = .9 + rd.y * s, wz = rd.z * s;
    float m = winMask(wy, wz, .03 + bl * .15);
    vec3 wc = mix(vec3(1., .55, .25), vec3(1., .86, .62), smoothstep(1., 6., wy)) * 11.;
    c += wc * m * smoothstep(-.12, -.3, rd.x) * mix(.05, 1., smoothstep(-.42, -.62, rd.x));
  }
  return c;
}

// ---------------------------------------------------------------- animation state
vec3 gCh; float gChVel; vec3 gStemD; float gLb;
vec4 gD[30]; float gSq[30]; int gN;
float gBub[6];

void initCherry(){
  gStemD = normalize(vec3(.3, 1., .12)); gLb = 0.; gChVel = 0.;
  if(uPre > .5){ gCh = vec3(0., 60., 0.); return; }
  float t = uT, ts = .35, g = 4.;
  if(t < uTI){
    float f = max(t - ts, 0.);
    float ys = Y0 + CR + .5 * g * (uTI - ts) * (uTI - ts);
    gCh = vec3(0., ys - .5 * g * f * f, 0.); gChVel = g * f; gLb = gChVel * .03;
  } else {
    float tau = t - uTI, z = 3.2, w = 9.;
    float B = exp(-z * tau) * (CR * cos(w * tau) + ((-2.2 + z * CR) / w) * sin(w * tau));
    float m = smoothstep(.05, .7, tau);
    gCh = vec3(-.075 * m, Y0 + B, .04 * m);
    gStemD = normalize(vec3(.3 + .5 * exp(-1.6 * tau) * sin(9. * tau), 1., .12));
  }
}
float waveH(vec2 q){
  float tt = uT - uTI;
  if(uPre > .5 || tt < 0.) return 0.;
  float fade = 1. - smoothstep(uDur - 1.3, uDur, uT);
  float r = length(q), c = .5, k = 2. * PI / .075;
  float front = c * tt;
  float e1 = smoothstep(0., .05, front - r) * exp(-max(front - r, 0.) * 4.);
  float a = .012 / sqrt(.5 + r * 6.);
  float w = sin(k * (r - front)) * e1 * a;
  float d2 = front - (.58 - r);
  float e2 = smoothstep(0., .05, d2) * exp(-max(d2, 0.) * 4.) * step(r, .3);
  w += sin(k * (.58 - r - front)) * e2 * a * .7;
  w *= exp(-1.5 * tt);
  w += .03 * q.x * sin(8. * tt) * exp(-1.8 * tt);
  w -= .06 * exp(-r * r / .0045) * exp(-tt * 3.5) * smoothstep(0., .04, tt);
  w += .03 * exp(-r * r / .004) * smoothstep(.2, .4, tt) * (1. - smoothstep(.55, 1., tt));
  float dcr = max(length(q - gCh.xz) - CR * .95, 0.);
  w += .011 * exp(-dcr / .016) * smoothstep(.35, .6, tt) * (1. - smoothstep(.04, .1, abs(gCh.y - Y0)));
  return w * fade;
}
float sdEll(vec3 p, vec3 c, vec3 r){ vec3 q = p - c; float k0 = length(q / r), k1 = length(q / (r * r)); return k0 * (k0 - 1.) / max(k1, 1e-5); }
float sdInner(vec3 p){ return sdEll(p, BC, BR - vec3(WALL)); }

void addDrop(vec3 c, float r, float sq){ if(r > .0005 && gN < 30){ gD[gN] = vec4(c, r); gSq[gN] = sq; gN++; } }
void initDrops(){
  gN = 0;
  for(int i = 0; i < 6; i++) gBub[i] = 0.;
  if(uPre > .5 || uT <= uTI) return;
  float tt = uT - uTI, G = 4.5;
  for(int i = 0; i < 24; i++){
    float fi = float(i);
    float h1 = h11(fi * 1.31 + 1.), h2 = h11(fi * 2.17 + 5.), h3 = h11(fi * 3.7 + 9.), h4 = h11(fi * 5.1 + 2.);
    float phi, r0, vh, vy, tl, rad, y0 = Y0 + .02; bool isC = false;
    if(i < 6){ phi = 6.2832 * (fi / 6.) + h1 * .5; r0 = .13; vh = .5 + .4 * h2; vy = 1.2 + 1.2 * h3; tl = .02 + .03 * h1; rad = .014 + .01 * h4; }
    else if(i < 16){ phi = 6.2832 * h1; r0 = .03 + .07 * h2; vh = .1 + .35 * h3; vy = 1.6 + 1.2 * h4; tl = .06 + .18 * h2; rad = .010 + .014 * h3; }
    else if(i < 21){ phi = .9 + 1.1 * h1; r0 = .23; y0 = 1.12; vh = .35 + .35 * h2; vy = 1.4 + 1.2 * h3; tl = .26 + .25 * h4; rad = .016 + .01 * h2; isC = true; }
    else { phi = 6.2832 * h1; r0 = .05; vh = .15 + .2 * h2; vy = 1.5 + .8 * h3; tl = .45 + .3 * h4; rad = .010; }
    float tau = tt - tl;
    if(tau < 0.) continue;
    float grow = smoothstep(0., .04, tau);
    vec3 p = vec3((r0 + vh * tau) * cos(phi), y0 + vy * tau - .5 * G * tau * tau, (r0 + vh * tau) * sin(phi));
    float sq = 0.;
    if(isC){ if(p.y < rad * .5){ p.y = rad * .45; sq = 1.; if(tau > .5) rad *= 1.15; } }
    else {
      if(p.y < 1.09 && sdInner(p) > -.015) continue;       // hits the glass wall: gone
      if(p.y < Y0 + .005) continue;                         // back in the wine
    }
    addDrop(p, rad * grow * 1.5, sq);
  }
  // Worthington jet
  float jt = tt - .10;
  if(jt > 0.){
    float u = clamp(jt / .9, 0., 1.);
    float H = .42 * pow(sin(PI * u), .8);
    float vis = smoothstep(.02, .1, H);
    for(int j = 0; j < 6; j++){
      float f = float(j) / 5.;
      float y = Y0 + H * f;
      float r = (.055 * (1. - .6 * f) + .014) * 1.2;
      if(j == 5){ y += .12 * smoothstep(.35, .8, jt); r = .022; }
      addDrop(vec3(.02 + .01 * f, y, -.02), r * vis, 0.);
    }
  }
  // bubbles rising from the cherry spot, settling at the surface
  for(int b = 0; b < 6; b++){
    float fb = float(b);
    float tb = tt - .08 - .09 * fb;
    if(tb < 0.){ gBub[b] = 0.; continue; }
    gBub[b] = 1.;
  }
}
float bubbleCut(vec3 p){
  float tt = uT - uTI;
  if(uPre > .5 || tt < .05) return 1e3;
  float d = 1e3;
  for(int b = 0; b < 6; b++){
    if(gBub[b] < .5) continue;
    float fb = float(b);
    float tb = tt - .08 - .09 * fb;
    float rb = .011 + .008 * h11(fb * 7.3);
    float ang = fb * 2.4;
    float rad0 = .02 + .03 * fb;
    float rest = (fb < 3.) ? .06 + .05 * fb : .17 + .03 * (fb - 3.);
    float rr = mix(rad0, rest, smoothstep(0., 1.4, tb));
    float yb = mix(Y0 - .2, Y0 - rb * .4, smoothstep(0., 1.3, tb));
    vec3 c = vec3(-.075 + rr * cos(ang), yb, .04 + rr * sin(ang));
    d = min(d, length(p - c) - rb);
  }
  return d;
}
float sdDrops(vec3 p){
  float d = 1e3;
  for(int i = 0; i < 30; i++){
    if(i >= gN) break;
    vec3 q = p - gD[i].xyz;
    float s = gSq[i];
    float e = length(q * vec3(1., 1. + 1.8 * s, 1.)) - gD[i].w;
    d = smin(d, e * (1. - .35 * s), .03);
  }
  return d;
}

// ---------------------------------------------------------------- geometry
float sdG(vec3 p){
  float o = smax(sdEll(p, BC, BR), p.y - 1.08, .012);
  float shell = max(o, -sdInner(p));
  float stem = max(length(p.xz) - .026, max(p.y - .40, .03 - p.y));
  float foot = max(sdEll(p, vec3(0.), vec3(.34, .07, .34)), -p.y);
  return smin(smin(shell, stem, .05), foot, .06);
}
float sdWineRaw(vec3 p){
  float inn = sdInner(p);
  float dw = max(-inn, 0.);
  float h = Y0 + waveH(p.xz) + .011 * exp(-dw / .012);
  float w = max(inn, (p.y - h) * .75);
  if(gN > 0) w = smin(w, sdDrops(p), .045);
  if(uPre < .5 && uT > uTI) w = max(w, -bubbleCut(p));
  return w;
}
float sdCherryObj(vec3 p, out float mat){
  float a = length(p - gCh) - CR;
  if(gLb > 0.001){ vec3 pa = p - gCh, ba = vec3(0., gLb, 0.); float h = clamp(dot(pa, ba) / dot(ba, ba), 0., 1.); a = length(pa - ba * h) - CR; }
  vec3 s0 = gCh + vec3(0., CR * .8 + gLb, 0.);
  vec3 pa = p - s0, ba = gStemD * .2;
  float h = clamp(dot(pa, ba) / dot(ba, ba), 0., 1.);
  float st = length(pa - ba * h) - (.0075 - .002 * h);
  mat = st < a ? 1. : 0.;
  return min(a, st);
}
vec3 sdfs(vec3 p){ float m; return vec3(sdG(p), sdWineRaw(p), sdCherryObj(p, m)); }
float dM(vec3 s, int med){ return med == 0 ? min(s.x, s.y) : (med == 1 ? -s.x : min(-s.y, s.x)); }
vec3 gradM(vec3 p, int med){
  const vec2 k = vec2(1., -1.); const float e = .0012;
  return normalize(k.xyy * dM(sdfs(p + k.xyy * e), med) + k.yyx * dM(sdfs(p + k.yyx * e), med) +
                   k.yxy * dM(sdfs(p + k.yxy * e), med) + k.xxx * dM(sdfs(p + k.xxx * e), med));
}
vec3 gradC(vec3 p){
  const vec2 k = vec2(1., -1.); const float e = .0008; float m;
  return normalize(k.xyy * sdCherryObj(p + k.xyy * e, m) + k.yyx * sdCherryObj(p + k.yyx * e, m) +
                   k.yxy * sdCherryObj(p + k.yxy * e, m) + k.xxx * sdCherryObj(p + k.xxx * e, m));
}
bool boundCyl(vec3 ro, vec3 rd, out float t0, out float t1){
  float R = (uPre < .5 && uT > uTI) ? 1.05 : .42;
  float a = dot(rd.xz, rd.xz), b = dot(ro.xz, rd.xz), c = dot(ro.xz, ro.xz) - R * R;
  float tn = 0., tf = 1e9;
  if(a > 1e-8){ float disc = b * b - a * c; if(disc < 0.) return false; float s = sqrt(disc); tn = (-b - s) / a; tf = (-b + s) / a; }
  else if(c > 0.) return false;
  if(abs(rd.y) > 1e-6){ float ya = (-.02 - ro.y) / rd.y, yb = (4.2 - ro.y) / rd.y; tn = max(tn, min(ya, yb)); tf = min(tf, max(ya, yb)); }
  else if(ro.y < -.02 || ro.y > 4.2) return false;
  t0 = max(tn, 0.); t1 = tf; return t1 > t0;
}

// ---------------------------------------------------------------- wood counter
vec3 woodAlb(vec2 q, float fw, out float hgt){
  float pl = floor(q.y / .64), py = q.y - pl * .64, sd = pl * 13.7 + 3.1;
  float px = q.x + h11(sd) * 20.;
  float warp = vn(vec2(px * .5, q.y * 2.5 + sd)) * 1.2 + vn(vec2(px * 1.7 + sd, q.y * 9.)) * .35;
  float rings = py * (10. + 4. * h11(sd + 1.)) + warp * 3. + sin(px * .45 + sd) * .7;
  float f = fract(rings);
  float ring = clamp(pow(f, 2.2) * (1. - f) * 4., 0., 1.);
  float fa = 1. - smoothstep(.004, .014, fw);
  float st = vn(vec2(px * 1.4, q.y * 55. + sd)) * .6 + vn(vec2(px * 3., q.y * 160.)) * .4;
  st = mix(.5, st, fa);
  float pore = 0.;
  if(fw < .006){ float pn = vn(vec2(px * 70., q.y * 900.)); pore = smoothstep(.72, .9, pn) * (1. - smoothstep(.002, .006, fw)); }
  float tone = .30 + .55 * ring + .45 * (st - .5) + .25 * (warp - .8);
  vec3 col = mix(vec3(.045, .02, .01), vec3(.20, .092, .037), clamp(tone, 0., 1.));
  col *= .8 + .45 * h11(sd + 5.);
  col *= 1. - .55 * pore;
  float seam = 1. - smoothstep(0., .005 + fw, min(py, .64 - py));
  col *= 1. - .8 * seam;
  hgt = (st - .5) * .3 * fa - pore * .6 - seam;
  return col;
}
void floorSurface(vec3 p, float fw, bool full, out vec3 alb, out vec3 n){
  float h0, h1, h2;
  alb = woodAlb(p.xz, fw, h0);
  n = vec3(0., 1., 0.);
  if(full){
    woodAlb(p.xz + vec2(.0025, 0.), fw, h1); woodAlb(p.xz + vec2(0., .0025), fw, h2);
    n = normalize(vec3(-(h1 - h0) * .35, 1., -(h2 - h0) * .35));
  }
  float ze = p.z - ZB;
  if(ze < .07){ float s = 1. - ze / .07; n = normalize(vec3(n.x, 1., n.z - s * .8)); alb *= 1. - .5 * s; }
}
vec3 glassShadow(vec3 p){
  vec3 acc = vec3(0.);
  float ph = h21(gl_FragCoord.xy);
  vec3 ri = BR - vec3(WALL);
  for(int k = 0; k < 4; k++){
    float a = (float(k) + ph) * 1.5708;
    vec2 j = vec2(cos(a), sin(a)) * (.035 * (.5 + .5 * fract(float(k) * .618 + ph)));
    vec3 ld = normalize(SUN + vec3(j.x, j.y * .5, j.y));
    vec3 q = (p - BC) / ri, d = ld / ri;
    float A = dot(d, d), B = dot(q, d), C = dot(q, q) - 1., disc = B * B - A * C;
    vec3 tr = vec3(1.);
    if(disc > 0.){
      float s = sqrt(disc), t0 = max((-B - s) / A, 0.), t1 = (-B + s) / A;
      float tcap = (Y0 - p.y) / ld.y;
      float chord = max(min(t1, tcap) - t0, 0.);
      tr = exp(-SIGW * chord) * .8;
    }
    float tlo = (.03 - p.y) / ld.y, thi = (.40 - p.y) / ld.y;
    vec2 pxz = p.xz, dxz = ld.xz;
    float tm = clamp(-dot(pxz, dxz) / max(dot(dxz, dxz), 1e-5), max(tlo, 0.), max(thi, 0.));
    float ds = length(pxz + dxz * tm);
    tr *= 1. - .5 * (1. - smoothstep(.016, .045, ds)) * step(tlo, thi);
    float tf = (.04 - p.y) / ld.y;
    if(tf > 0. && length(p.xz + ld.xz * tf) < .33) tr *= .72;
    acc += tr;
  }
  return acc * .25;
}
float causticShape(vec3 p){
  vec2 a = normalize(-SUN.xz);
  vec2 c0 = BC.xz - SUN.xz / SUN.y * .76 - a * .10;
  float tt = uT - uTI;
  float wob = (uPre > .5 || tt < 0.) ? 0. : exp(-1.3 * tt) * sin(9. * tt) * (1. - smoothstep(uDur - 1.3, uDur, uT));
  c0 += a * .03 * wob;
  vec2 d = p.xz - c0;
  float da = dot(d, a), dc = dot(d, vec2(-a.y, a.x));
  float core = exp(-pow(da / .17, 2.) - pow(dc / (.085 + .03 * wob), 2.));
  float edge = .35 * exp(-pow((length(vec2(da / .32, dc / .2)) - 1.) / .14, 2.));
  return core * 1.3 + edge * .6 + .25 * exp(-pow(da / .5, 2.) - pow(dc / .22, 2.));
}
vec3 floorLight(vec3 p, vec3 rd, vec3 alb, vec3 n, vec3 R){
  float vis = sunMask(p) * art(p.x);
  vec3 tr = glassShadow(p);
  float ndl = max(dot(n, SUN), 0.);
  vec3 sunE = SUNC * 4.2;
  float ao = 1. - .6 * exp(-max(length(p.xz) - .30, 0.) / .09);
  float caus = causticShape(p) * vis;
  vec3 irr = sunE * ndl * vis * tr + sunE * ndl * caus * vec3(1., .3, .1) * 2.4 + vec3(.05, .075, .105) * (1. - ndl * .0) + vec3(.045, .022, .012);
  vec3 diff = alb * irr * ao;
  float cv = max(-rd.y, .03);
  float F = .045 + .955 * pow(1. - cv, 5.);
  vec3 col = diff * (1. - F) + R * F * vec3(1., .97, .92) * mix(.5, 1., ao);
  vec3 H = normalize(SUN - rd);
  float nh = max(dot(n, H), 0.), a2 = .0144;
  float D = a2 / (PI * pow(nh * nh * (a2 - 1.) + 1., 2.));
  col += sunE * vis * tr * D * F * .25 * ndl;
  return col;
}

// ---------------------------------------------------------------- cherry
vec3 cherryShade(vec3 p, vec3 n, vec3 rd, bool stem){
  vec3 alb = stem ? vec3(.10, .085, .025) : vec3(.30, .008, .016);
  float vis = sunMask(p) * art(p.x);
  if(p.y < Y0) vis *= .2;
  float ndl = max(dot(n, SUN), 0.);
  vec3 col = alb * (SUNC * 5. * ndl * vis + vec3(.06, .08, .11) + vec3(.05, .025, .015) * (.5 + .5 * n.y));
  float nv = max(dot(n, -rd), 0.);
  float F = .05 + .95 * pow(1. - nv, 5.);
  col += env(reflect(rd, n), .15) * F * (stem ? .25 : 1.4);
  vec3 H = normalize(SUN - rd); float nh = max(dot(n, H), 0.);
  col += SUNC * 5. * vis * (pow(nh, 350.) * 2. + pow(nh, 45.) * .15) * (stem ? .2 : 1.);
  col += vec3(.5, .02, .03) * pow(1. - nv, 3.) * .3 * (stem ? 0. : .5 + vis);
  return col;
}
vec3 reflCherry(vec3 ro, vec3 rd, out float hit){
  hit = 0.;
  vec3 oc = ro - gCh; float b = dot(oc, rd), c = dot(oc, oc) - CR * CR, d = b * b - c;
  if(d < 0. || gCh.y > 20.) return vec3(0.);
  float t = -b - sqrt(d); if(t < 0.) return vec3(0.);
  hit = 1.;
  vec3 p = ro + rd * t; vec3 n = normalize(p - gCh);
  return cherryShade(p, n, rd, false) * .9;
}

// ---------------------------------------------------------------- tracing through glass / wine
vec3 trace(vec3 ro, vec3 rd, bool primary, out float tFirst, out int status, out vec3 fpos, float pix){
  vec3 col = vec3(0.), T = vec3(1.);
  int med = 0; status = 0; tFirst = 30.;
  float tacc = 0.;
  for(int seg = 0; seg < 6; seg++){
    float tf = (med == 0 && rd.y < -1e-4) ? -ro.y / rd.y : 1e9;
    bool validF = tf < 1e8 && (ro.z + rd.z * tf) > ZB;
    float tEnd = validF ? tf : 1e9;
    float t = 0., t0 = 0., t1 = 1e9;
    bool doMarch = true;
    if(med == 0){ doMarch = boundCyl(ro, rd, t0, t1); if(doMarch){ t = t0; tEnd = min(tEnd, t1); } }
    else { tEnd = 4.; }
    bool hit = false, hitC = false;
    if(doMarch && t < tEnd){
      for(int i = 0; i < 56; i++){
        vec3 s = sdfs(ro + rd * t);
        float d = dM(s, med);
        if(s.z < .0005 && s.z <= d){ hitC = true; break; }
        if(d < .0005){ hit = true; break; }
        t += min(d, s.z) * .85;
        if(t >= tEnd) break;
      }
    }
    float segLen = (hit || hitC) ? t : (doMarch ? min(t, tEnd) : tEnd);
    if(med == 2) T *= exp(-SIGW * min(segLen, 2.));
    if(med == 1) T *= exp(-SIGG * min(segLen, 2.));
    if(seg == 0) tFirst = (hit || hitC) ? t : (validF ? tf : 30.);
    if(!hit && !hitC){
      if(med == 0){
        if(validF){
          vec3 pf = ro + rd * tf;
          if(primary && seg == 0){ status = 1; fpos = pf; return col; }
          vec3 alb, n; floorSurface(pf, .01, false, alb, n);
          col += T * floorLight(pf, rd, alb, n, env(reflect(rd, n), .6));
        } else col += T * env(rd, 0.);
      }
      break;
    }
    vec3 p = ro + rd * t;
    if(hitC){
      float m; sdCherryObj(p, m);
      vec3 n = gradC(p);
      vec3 cc = cherryShade(p, n, rd, m > .5);
      float al = 1.;
      if(gLb > .001 && m < .5) al = mix(1., .12, smoothstep(0., 1., (p.y - gCh.y) / gLb));
      col += T * (al * cc + (1. - al) * env(rd, 0.));
      break;
    }
    if(med == 2 && (p.y > Y0 + .03 || sdInner(p) > 0.)) col += T * SUNC * 4. * vec3(.75, .03, .03) * (.25 + .75 * sunMask(p) * art(p.x)) * (1. - exp(-segLen * 35.)) * 1.1;
    vec3 n = gradM(p, med);
    vec3 pp = p - n * .004;
    vec3 sp = sdfs(pp);
    int nm = sp.x < 0. ? 1 : (max(sp.y, -sp.x) < 0. ? 2 : 0);
    if(nm == med){ ro = p + rd * .003; continue; }
    float n1 = med == 0 ? 1. : (med == 1 ? 1.5 : 1.34);
    float n2 = nm == 0 ? 1. : (nm == 1 ? 1.5 : 1.34);
    float cosi = clamp(-dot(rd, n), 0., 1.);
    float eta = n1 / n2;
    float k = 1. - eta * eta * (1. - cosi * cosi);
    vec3 rr = reflect(rd, n);
    float F;
    if(k < 0.) F = 1.;
    else { float F0 = pow((n1 - n2) / (n1 + n2), 2.); float cc = n1 > n2 ? sqrt(k) : cosi; F = F0 + (1. - F0) * pow(1. - cc, 5.); }
    if(med == 0){
      float ch; vec3 cr = reflCherry(p, rr, ch);
      vec3 re = ch > .5 ? cr : env(rr, 0.);
      col += T * F * re;
    }
    if(k < 0.){ rd = rr; ro = p + rd * .003; continue; }
    T *= (1. - F);
    rd = refract(rd, n, eta);
    ro = p + rd * .0025;
    med = nm;
  }
  return col;
}

vec3 floorFull(vec3 p, vec3 rd, float fw){
  vec3 alb, n; floorSurface(p, fw, true, alb, n);
  vec3 rr = reflect(rd, n);
  rr.y = max(rr.y, .02);
  float tf; int st; vec3 fp;
  vec3 R = trace(p + n * .001, normalize(rr), false, tf, st, fp, fw);
  return floorLight(p, rd, alb, n, R);
}

// ---------------------------------------------------------------- air: shafts and dust
vec3 hazeLight(vec3 ro, vec3 rd, float tmax, float jit){
  float tm = min(tmax, 11.);
  const int N = 28;
  float dt = tm / float(N);
  float acc = 0.;
  for(int i = 0; i < N; i++){
    float t = (float(i) + jit) * dt;
    vec3 p = ro + rd * t;
    if(p.y < 0.) continue;
    float m = sunMask(p);
    float dens = (.15 + 1.3 * vn3(p * vec3(1.2, .9, 1.2) + vec3(0., uT * .02, uT * .03))) * exp(-p.y * .22);
    acc += dens * m * art(p.x) * dt;
  }
  float g = .45, cs = dot(rd, -SUN);
  float ph = (1. - g * g) / pow(1. + g * g - 2. * g * cs, 1.5) * .08;
  return SUNC * 5.2 * acc * .04 * (.35 + ph * 2.);
}
vec3 motes(vec3 ro, vec3 rd, float tmax){
  vec3 acc = vec3(0.);
  for(int l = 0; l < 7; l++){
    float dl = 2.2 + float(l) * .55;
    if(dl > tmax) break;
    vec3 p = ro + rd * dl;
    float cs = .32;
    vec3 g = p / cs + vec3(0., uT * .06, 0.);
    vec3 id = floor(g);
    vec3 r = vec3(h31(id + float(l) * 7.1), h31(id + 13.7), h31(id + 4.3));
    if(r.x > .6) continue;
    vec3 c = (id + .15 + .7 * vec3(h31(id + 2.2), h31(id + 8.8), h31(id + 5.5))) * cs - vec3(0., uT * .06 * cs, 0.);
    c.x += .02 * sin(uT * .8 + r.y * 20.); c.z += .02 * cos(uT * .7 + r.z * 20.);
    float dist = length(cross(rd, c - ro));
    float rm = min(.0025 + .0015 * r.z + abs(dl - 3.7) * .0012, .005);
    float lit = sunMask(c) * art(c.x);
    float tw = .6 + .4 * sin(uT * 3. + r.y * 40.);
    acc += SUNC * 5. * lit * tw * exp(-pow(dist / rm, 2.)) * pow(.0025 / rm, 2.) * .5;
  }
  return acc;
}

// ---------------------------------------------------------------- camera / per-pixel
vec3 camRay(vec2 fc){
  vec3 f = normalize(vec3(0., .62, 0.) - CAM);
  vec3 r = normalize(cross(f, vec3(0., 1., 0.))), u = cross(r, f);
  vec2 uv = fc / uRes; float asp = uRes.x / uRes.y;
  float tv = TANV * max(1., .95 / asp);
  return normalize(f + r * ((uv.x - .72) * 2. * tv * asp) + u * ((uv.y - .40) * 2. * tv));
}
vec3 shade(vec2 fc){
  vec3 rd = camRay(fc), ro = CAM;
  float tv = TANV * max(1., .95 / (uRes.x / uRes.y));
  float pix = 2. * tv / uRes.y;
  float tF; int st; vec3 fp;
  vec3 col = trace(ro, rd, true, tF, st, fp, pix);
  if(st == 1){
    tF = length(fp - ro);
    float fw = pix * tF / max(abs(rd.y), .05);
    col = floorFull(fp, rd, fw);
  }
  float jit = h21(fc + 7.3);
  col = col * exp(-.012 * min(tF, 12.)) + hazeLight(ro, rd, tF, jit) + motes(ro, rd, tF);
  return col;
}
bool hitEll(vec3 ro, vec3 rd, vec3 c, vec3 r){
  vec3 q = (ro - c) / r, d = rd / r;
  float A = dot(d, d), B = dot(q, d), C = dot(q, q) - 1.;
  return B * B - A * C > 0. && (-B + sqrt(B * B - A * C)) > 0.;
}
int cls(vec2 fc){
  vec3 rd = camRay(fc), ro = CAM; int m = 0;
  if(hitEll(ro, rd, BC, BR)) m |= 1;
  if(hitEll(ro, rd, BC, BR - vec3(WALL))) m |= 2;
  if(hitEll(ro, rd, vec3(0.), vec3(.34, .07, .34))) m |= 4;
  { vec2 o = ro.xz, d = rd.xz; float A = dot(d, d), B = dot(o, d), C = dot(o, o) - .026 * .026, disc = B * B - A * C;
    if(disc > 0.){ float t = (-B - sqrt(disc)) / A; float y = ro.y + rd.y * t; if(y > .03 && y < .4) m |= 8; } }
  if(gCh.y < 20.){ vec3 oc = ro - gCh; float b = dot(oc, rd), c = dot(oc, oc) - (CR + .012) * (CR + .012); if(b * b - c > 0.) m |= 16;
    vec3 s0 = gCh + vec3(0., CR * .8 + gLb, 0.); vec3 pa = ro - s0, ba = gStemD * .2; float hh;
    vec3 w = cross(rd, ba); float den = dot(w, w); if(den > 1e-8){ float dd = abs(dot(pa, w)) / sqrt(den); if(dd < .012) m |= 32; } }
  if(rd.y < 0.){ float tf = -ro.y / rd.y; if(ro.z + rd.z * tf > ZB) m |= 64; if(ro.z + rd.z * tf > ZB + .07) m |= 128; }
  for(int i = 0; i < 30; i++){ if(i >= gN) break; vec3 oc = ro - gD[i].xyz; float b = dot(oc, rd), c = dot(oc, oc) - pow(gD[i].w * 1.6 + .01, 2.); if(b * b - c > 0.) { m |= 256; break; } }
  if(uPre < .5 && uT > uTI && uT < uTI + 2.4){ float t0, t1; if(boundCyl(ro, rd, t0, t1)) m |= 512; }
  return m;
}
void main(){
  initCherry(); initDrops();
  vec2 fc = gl_FragCoord.xy;
  int c0 = cls(fc + vec2(-.5, -.5)), c1 = cls(fc + vec2(.5, -.5)), c2 = cls(fc + vec2(-.5, .5)), c3 = cls(fc + vec2(.5, .5));
  vec3 col;
  if(c0 == c1 && c1 == c2 && c2 == c3 && (c0 & 512) == 0) col = shade(fc);
  else col = (shade(fc + vec2(-.25, -.125)) + shade(fc + vec2(.125, -.25)) + shade(fc + vec2(.25, .125)) + shade(fc + vec2(-.125, .25))) * .25;
  fragColor = vec4(min(col, vec3(60.)) * uHdr, 1.);
}`;

const FS_POST = `#version 300 es
precision highp float;
uniform sampler2D uTex;
uniform vec2 uRes;
uniform float uGrain;
uniform float uHdr;
out vec4 o;
float h21(vec2 p){ vec3 p3 = fract(vec3(p.xyx) * .1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
vec3 aces(vec3 x){ return clamp((x * (2.51 * x + .03)) / (x * (2.43 * x + .59) + .14), 0., 1.); }
vec3 srgb(vec3 c){ return mix(c * 12.92, 1.055 * pow(c, vec3(1. / 2.4)) - .055, step(.0031308, c)); }
void main(){
  vec2 uv = gl_FragCoord.xy / uRes;
  vec2 cc = uv - vec2(.72, .42);
  float ca = .0016;
  vec3 col = vec3(texture(uTex, uv + cc * ca).r, texture(uTex, uv).g, texture(uTex, uv - cc * ca).b) / uHdr;
  vec3 b = vec3(0.);
  vec2 ts = vec2(textureSize(uTex, 0));
  for(int L = 2; L <= 6; L++){
    float fl = float(L); vec2 px = pow(2., fl) / ts;
    vec3 s = textureLod(uTex, uv, fl).rgb + textureLod(uTex, uv + vec2(px.x, 0.) * .7, fl).rgb + textureLod(uTex, uv - vec2(px.x, 0.) * .7, fl).rgb
           + textureLod(uTex, uv + vec2(0., px.y) * .7, fl).rgb + textureLod(uTex, uv - vec2(0., px.y) * .7, fl).rgb;
    s = s * .2 / uHdr;
    b += max(s - .7, 0.) * (1.1 - fl * .1);
  }
  col += b * .09;
  float lum = dot(col, vec3(.3, .59, .11));
  col += vec3(.0015, .004, .007) * (1. - smoothstep(0., .4, lum));
  col = aces(col * 1.12);
  float v = smoothstep(1.3, .3, length((uv - vec2(.56, .5)) * vec2(1.05, 1.)));
  col *= mix(.5, 1., v);
  col = srgb(col);
  col += (h21(gl_FragCoord.xy) + h21(gl_FragCoord.xy + 17.1) - 1.) * .022 * uGrain * (.5 + .5 * (1. - lum));
  o = vec4(col, 1.);
}`;

export function createScene({ width, height, quality = 1, grain = 1, tileRows = 0 } = {}) {
  const canvas = document.createElement("canvas");
  canvas.width = width; canvas.height = height;
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true, antialias: false, alpha: false, depth: false, stencil: false, powerPreference: "high-performance" });
  if (!gl) return null;
  const par = gl.getExtension("KHR_parallel_shader_compile");
  const hdrOK = !!(gl.getExtension("EXT_color_buffer_float") || gl.getExtension("EXT_color_buffer_half_float"));
  const hdr = hdrOK ? 1 : 0.25;
  const timer = gl.getExtension("EXT_disjoint_timer_query_webgl2");
  let W = width, H = height, scale = 1, ema = 0, lastCpu = 0, q = null, disposed = false;
  let fbo = null, tex = null, tw = 0, th = 0;

  const mk = (fs) => {
    const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; };
    const p = gl.createProgram(), v = sh(gl.VERTEX_SHADER, VS), f = sh(gl.FRAGMENT_SHADER, fs);
    gl.attachShader(p, v); gl.attachShader(p, f); gl.linkProgram(p);
    return { p, v, f };
  };
  const A = mk(FS), B = mk(FS_POST);
  const done = (x) => !par || gl.getProgramParameter(x.p, par.COMPLETION_STATUS_KHR);
  let ready_ = false, uA = {}, uB = {};
  const ready = new Promise((resolve) => {
    const poll = () => {
      if (disposed) return resolve(false);
      if (!done(A) || !done(B)) return void setTimeout(poll, 4);
      const ok = gl.getProgramParameter(A.p, gl.LINK_STATUS) && gl.getProgramParameter(B.p, gl.LINK_STATUS);
      if (!ok) {
        console.warn("scene3d link failed:", gl.getShaderInfoLog(A.f), gl.getProgramInfoLog(A.p), gl.getShaderInfoLog(B.f), gl.getProgramInfoLog(B.p));
        return resolve(false);
      }
      for (const n of ["uRes", "uT", "uPre", "uTI", "uDur", "uHdr"]) uA[n] = gl.getUniformLocation(A.p, n);
      for (const n of ["uTex", "uRes", "uGrain", "uHdr"]) uB[n] = gl.getUniformLocation(B.p, n);
      ready_ = true; resolve(true);
    };
    setTimeout(poll, 0);
  });

  const ensureTarget = (w, h) => {
    if (tex && tw === w && th === h) return;
    if (tex) { gl.deleteTexture(tex); gl.deleteFramebuffer(fbo); }
    tex = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, hdrOK ? gl.RGBA16F : gl.RGBA8, w, h, 0, gl.RGBA, hdrOK ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    fbo = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    tw = w; th = h;
  };

  function render(t) {
    if (!ready_ || disposed) return;
    const pre = t < 0, tt = pre ? 0 : Math.min(t, DURATION);
    if (quality === "auto") {
      if (timer && q) {
        if (gl.getQueryParameter(q, gl.QUERY_RESULT_AVAILABLE) && !gl.getParameter(timer.GPU_DISJOINT_EXT)) {
          const ms = gl.getQueryParameter(q, gl.QUERY_RESULT) / 1e6; ema = ema ? ema * .8 + ms * .2 : ms; q = null;
          scale = Math.min(1, Math.max(.5, scale * Math.pow(10 / Math.max(ema, 1), .25)));
        }
      } else if (!timer) {
        const now = performance.now(); if (lastCpu) { const d = now - lastCpu; ema = ema ? ema * .85 + d * .15 : d; if (ema > 20) scale = Math.max(.5, scale * .97); else if (ema < 17.5) scale = Math.min(1, scale * 1.01); } lastCpu = now;
      }
    } else scale = typeof quality === "number" ? Math.min(1, Math.max(.25, quality)) : 1;
    const rw = Math.max(8, Math.round(W * scale)), rh = Math.max(8, Math.round(H * scale));
    ensureTarget(rw, rh);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo); gl.viewport(0, 0, rw, rh);
    gl.useProgram(A.p);
    gl.uniform2f(uA.uRes, rw, rh); gl.uniform1f(uA.uT, tt); gl.uniform1f(uA.uPre, pre ? 1 : 0);
    gl.uniform1f(uA.uTI, T_IMPACT); gl.uniform1f(uA.uDur, DURATION); gl.uniform1f(uA.uHdr, hdr);
    let tq = null;
    if (quality === "auto" && timer && !q) { tq = gl.createQuery(); gl.beginQuery(timer.TIME_ELAPSED_EXT, tq); }
    if (tileRows > 0) {
      gl.enable(gl.SCISSOR_TEST);
      for (let y = 0; y < rh; y += tileRows) { gl.scissor(0, y, rw, Math.min(tileRows, rh - y)); gl.drawArrays(gl.TRIANGLES, 0, 3); gl.finish(); }
      gl.disable(gl.SCISSOR_TEST);
    } else gl.drawArrays(gl.TRIANGLES, 0, 3);
    if (tq) { gl.endQuery(timer.TIME_ELAPSED_EXT); q = tq; }
    gl.bindTexture(gl.TEXTURE_2D, tex); gl.generateMipmap(gl.TEXTURE_2D);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null); gl.viewport(0, 0, W, H);
    gl.useProgram(B.p);
    gl.activeTexture(gl.TEXTURE0); gl.uniform1i(uB.uTex, 0);
    gl.uniform2f(uB.uRes, W, H); gl.uniform1f(uB.uGrain, grain); gl.uniform1f(uB.uHdr, hdr);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
  function resize(w, h) { W = w; H = h; canvas.width = w; canvas.height = h; }
  function dispose() { disposed = true; try { gl.getExtension("WEBGL_lose_context")?.loseContext(); } catch {} }
  return { canvas, ready, render, resize, dispose, get scale() { return scale; } };
}
