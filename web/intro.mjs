// Plonk boot intro: a glass ball rolls in from the left and rests 3/4 of the way across. Ray-traced analytically in one
// full-screen fragment shader. The final frame (t >= DURATION) is the desktop wallpaper (see make_wallpaper.mjs).
export const DURATION = 4.0; // seconds of the roll (ease-out)

const X_REST = 0.75;
// Ball centre in screen pixels + radius at time t. Rolls without slipping: angle = distance / radius.
export function ballState(w, h, t) {
  const r = 0.17 * h, x0 = -1.2 * r, x1 = X_REST * w;
  const k = Math.min(Math.max(t / DURATION, 0), 1);
  const s = 1 - Math.pow(1 - k, 3);
  const x = x0 + (x1 - x0) * s;
  return { x, y: 0.71 * h, r, theta: (x - x0) / r };
}

const VS = `#version 300 es
void main(){ vec2 p = vec2(float((gl_VertexID<<1)&2), float(gl_VertexID&2)); gl_Position = vec4(p*2.0-1.0, 0.0, 1.0); }`;

const FS = `#version 300 es
precision highp float;
uniform vec2 uRes; uniform float uCx; uniform float uTheta;
out vec4 outc;

float W, H, R, F, HY, YF, ZW;   // pixel units; camera at origin, +z forward, +y down, horizon at screen y=HY
vec3 C; vec3 L;                 // sphere centre, direction toward key light

// ---- backdrop: identical to display::wallpaper() ----
vec3 wp(vec2 s){
  vec2 uv = s / uRes;
  vec3 a0 = vec3(96.,128.,220.)/255., a1 = vec3(168.,128.,226.)/255., a2 = vec3(240.,156.,190.)/255.;
  float t = clamp(uv.x*.55 + uv.y*.45, 0., 1.);
  vec3 c = t < .55 ? mix(a0, a1, t/.55) : mix(a1, a2, (t-.55)/.45);
  vec2 b0 = vec2(.22,.78), b1 = vec2(.82,.22);
  float d0 = length((uv-b0)*uRes)/(.35*W), d1 = length((uv-b1)*uRes)/(.30*W);
  float g0 = pow(clamp(1.-d0,0.,1.),2.)*.55, g1 = pow(clamp(1.-d1,0.,1.),2.)*.55;
  c = mix(c, vec3(60.,210.,230.)/255., g0);
  c = mix(c, vec3(255.,200.,120.)/255., g1);
  return c;
}
vec2 wallS(vec3 o, vec3 d){ float t = (ZW-o.z)/d.z; vec3 p = o + d*t; return vec2(.5*W + p.x*F/ZW, HY + p.y*F/ZW); }

float box(vec3 r, vec3 c, float hw, float hh){
  float dz = dot(r,c); if (dz <= 0.) return 0.;
  vec3 rt = normalize(cross(vec3(0.,1.,0.), c)); vec3 up = cross(c, rt);
  float a = dot(r,rt)/dz, b = dot(r,up)/dz;
  return (1.-smoothstep(hw*.8,hw,abs(a))) * (1.-smoothstep(hh*.8,hh,abs(b)));
}
// procedural studio: horizon gradient, soft boxes, window with mullions
vec3 studio(vec3 r){
  float y = -r.y;
  vec3 sky = mix(vec3(.84,.85,.97), vec3(.50,.57,.84), smoothstep(0.,.95,y));
  vec3 flo = mix(vec3(.58,.57,.72), vec3(.16,.16,.28), smoothstep(0.,.85,-y));
  vec3 c = mix(flo, sky, smoothstep(-.06,.06,y));
  c += vec3(1.,.92,.86) * .18 * exp(-y*y*60.);
  vec3 kc = normalize(L);
  c += vec3(1.,.97,.92) * 3.4 * box(r, kc, .42, .30);
  c += vec3(.86,.93,1.) * 2.3 * box(r, normalize(vec3(.78,-.22,-.6)), .12, .75);
  vec3 wc = normalize(vec3(.12,-.95,-.3));
  float wb = box(r, wc, .55, .22);
  vec3 rt = normalize(cross(vec3(0.,1.,0.), wc)); float a = dot(r,rt)/max(dot(r,wc),.01);
  float mull = smoothstep(.035,.015,abs(fract(a*2.2+.5)-.5)/2.2*2.2);
  c += vec3(.95,.98,1.) * 1.5 * wb * (1.-mull);
  return c;
}

float shadowAt(vec3 P){
  vec3 v = C-P; float tc = dot(v,L); float sh = 0.;
  if (tc > 0.){
    float dist = length(v - L*tc), pen = R*.10 + tc*.10;
    float occ = 1. - smoothstep(R-pen, R+pen, dist);
    sh = occ * (.62 - .30*exp(-dist*dist/(.55*R*.55*R)));
  }
  float dxz = length(P.xz - C.xz);
  float ao = .50*exp(-pow(dxz/(.70*R),2.)) + .35*exp(-pow(dxz/(.30*R),2.));
  return 1. - (1.-sh)*(1.-ao);
}

vec3 scene(vec3 o, vec3 d){
  vec3 dd = normalize(vec3(d.xy, max(d.z,.05)));
  float wt = (ZW-o.z)/dd.z;
  float ft = dd.y > .001 ? (YF-o.y)/dd.y : 1e9;
  vec3 col;
  vec2 ws = wallS(o, dd);
  vec3 wall = wp(ws);
  if (ft > 0. && ft < wt){
    vec3 P = o + dd*ft;
    vec2 sp = vec2(.5*W + P.x*F/P.z, HY + P.y*F/P.z);
    vec3 base = wp(sp) * vec3(.80,.81,.88) + vec3(.02,.025,.05);
    base *= 1. - .10*smoothstep(HY, H, sp.y);
    vec3 rd = vec3(dd.x, -dd.y, dd.z);
    vec3 refl = wp(wallS(P, rd));
    float cs = clamp(dd.y,0.,1.);
    float fres = .04 + .96*pow(1.-cs, 5.);
    vec3 fl = mix(base, refl, clamp(fres*.9 + .06, 0., 1.));
    // faint reflection of the ball itself
    vec3 oc = P - C; float bb = dot(oc,rd), cc = dot(oc,oc) - R*R, dsc = bb*bb - cc;
    if (dsc > 0. && -bb - sqrt(dsc) > 0.){
      vec3 hp = P + rd*(-bb - sqrt(dsc)); vec3 n = (hp-C)/R;
      vec3 bc = studio(reflect(rd,n))*.45 + vec3(.45,.58,.72)*.35 + wp(sp)*.25;
      float edge = smoothstep(0.,.35, sqrt(dsc)/R);
      fl = mix(fl, bc, .20*(1.-.5*smoothstep(0.,.6,(hp.y-C.y)/R+.5))*mix(.5,1.,edge));
    }
    float sh = shadowAt(P);
    fl *= 1. - sh*vec3(.50,.47,.38);
    // caustic: light focused through the ball, opposite the key light
    float s = (YF - C.y)/(-L.y); vec3 S = C - L*s;
    vec3 q = P - S; q.x *= .85;
    float cau = exp(-dot(q,q)/(2.*(.20*R)*(.20*R)));
    float halo = exp(-pow((length(q)-.34*R)/(.10*R),2.)) * .22;
    fl += vec3(1.,.96,.88) * (cau*.55 + halo*.35) * smoothstep(.1,.5,sh+.2);
    float far = smoothstep(.30*ZW, 1.0*ZW, P.z);
    col = mix(fl, wall, far);
  } else col = wall;
  float kb = smoothstep(.12,-.18,d.z);
  return mix(col, studio(d), kb);
}

vec3 rotL(vec3 v, float c, float s){ return vec3(v.x*c + v.y*s, -v.x*s + v.y*c, v.z); }

// refracted path through the (solid) sphere for one IOR; returns colour
vec3 trace(vec3 p0, vec3 n, vec3 d, float ior, float cth, float sth){
  vec3 r1 = refract(d, n, 1./ior);
  float t1 = -2.*dot(p0-C, r1);
  vec3 p1 = p0 + r1*t1; vec3 n1 = (p1-C)/R;
  float len = t1;
  vec3 r2 = refract(r1, -n1, ior);
  vec3 col;
  if (dot(r2,r2) < .5){
    vec3 rr = reflect(r1, -n1);
    float t2 = -2.*dot(p1-C, rr);
    vec3 p2 = p1 + rr*t2; vec3 n2 = (p2-C)/R; len += t2;
    vec3 r3 = refract(rr, -n2, ior);
    col = dot(r3,r3) < .5 ? studio(reflect(rr,-n2))*.5 : scene(p2, r3);
  } else col = scene(p1, r2);
  col *= exp(-(len/R) * vec3(.17,.05,.06));
  col *= vec3(.95,1.,1.);
  // embedded rolling features in sphere-local coordinates (units of R)
  vec3 a = rotL((p0-C)/R, cth, sth), dl = rotL(r1, cth, sth);
  float L1 = t1/R;
  vec3 nr = normalize(vec3(.62,.35,.70));
  float dn = dot(dl, nr);
  if (abs(dn) > .02){
    float tt = -dot(a, nr)/dn;
    if (tt > 0. && tt < L1){
      vec3 q = a + dl*tt; float rho = length(q);
      float ring = smoothstep(.46,.49,rho) * (1.-smoothstep(.55,.58,rho));
      float ang = atan(dot(q, normalize(cross(nr, vec3(0.,0.,1.)))), dot(q, normalize(cross(nr, cross(nr, vec3(0.,0.,1.))))));
      ring *= .55 + .45*sin(ang*2. + 1.);
      col = mix(col, vec3(.70,.95,1.)*1.05 + vec3(.2,.0,.1)*(.5+.5*sin(ang*2.+1.)), ring*.32);
    }
  }
  vec3 bp0 = vec3(.30,-.22,.10), bp1 = vec3(-.36,.26,-.2), bp2 = vec3(.08,.42,.3), bp3 = vec3(-.14,-.42,.2), bp4 = vec3(.46,.14,-.25), bp5 = vec3(-.05,.02,-.46);
  float bm = 0.;
  vec3 bs[6] = vec3[6](bp0,bp1,bp2,bp3,bp4,bp5);
  for (int i = 0; i < 6; i++){
    float tb = clamp(dot(bs[i]-a, dl), 0., L1);
    float db = length(a + dl*tb - bs[i]);
    float br = .032;
    bm += smoothstep(br*1.25, br*.85, db)*.45 - smoothstep(br*.8, br*.25, db)*.30;
  }
  col += vec3(.9,.97,1.) * bm * .6;
  return col;
}

vec3 shade(vec2 sp){
  vec3 d = normalize(vec3(sp.x-.5*W, sp.y-HY, F));
  float b = dot(-C, d), c = dot(C,C) - R*R, dsc = b*b - c;
  if (dsc <= 0.) return scene(vec3(0.), d);
  float t = -b - sqrt(dsc);
  vec3 p = d*t; vec3 n = (p-C)/R;
  float cosi = clamp(-dot(d,n), 0., 1.);
  float fr = .04 + .96*pow(1.-cosi, 5.);
  vec3 refl = scene(p, reflect(d,n));
  float cth = cos(uTheta), sth = sin(uTheta);
  vec3 tr = vec3(trace(p,n,d,1.490,cth,sth).r, trace(p,n,d,1.505,cth,sth).g, trace(p,n,d,1.522,cth,sth).b);
  tr *= 1. - .85*smoothstep(.45,1., 1.-cosi);                 // inner rim darkening
  vec3 col = tr*(1.-fr) + refl*fr*1.15;
  float cres = smoothstep(.40,.92, dot(n, -L)) * smoothstep(.2,.8, cosi);   // focused light inside, opposite the key
  col += vec3(1.,.93,.80) * cres * .30;
  col *= 1. - .16*smoothstep(.2,-.6, dot(n, -L)) ;                          // slightly deeper towards the key side
  vec3 r = reflect(d,n);
  float sp1 = pow(max(dot(r, normalize(L)), 0.), 600.) * 4.;     // crisp key highlight
  float sp2 = pow(max(dot(r, normalize(vec3(.78,-.22,-.6))), 0.), 300.) * 1.6;
  col += vec3(1.,.98,.95)*sp1 + vec3(.85,.93,1.)*sp2;
  return col;
}

float knee(float x){ return x < .95 ? x : .95 + .05*(1.-exp(-(x-.95)/.05)); }

void main(){
  W = uRes.x; H = uRes.y; R = .17*H; F = 1.2*H; HY = .76*H; YF = .12*H; ZW = 3.*F;
  C = vec3(uCx - .5*W, YF - R, F);
  L = normalize(vec3(-.5,-.8,-.5));
  vec2 sp = vec2(gl_FragCoord.x, H - gl_FragCoord.y);
  vec3 d0 = normalize(vec3(sp.x-.5*W, sp.y-HY, F));
  float dist = length(cross(d0, C));
  vec3 col;
  if (abs(dist - R) < 2.5){
    col  = shade(sp + vec2(-.375,-.125)) + shade(sp + vec2(.125,-.375));
    col += shade(sp + vec2(.375,.125)) + shade(sp + vec2(-.125,.375));
    col *= .25;
  } else col = shade(sp);
  outc = vec4(knee(col.r), knee(col.g), knee(col.b), 1.);
}`;

export function createIntro(w, h) {
  let canvas;
  try { canvas = document.createElement("canvas"); } catch { return null; }
  canvas.width = w; canvas.height = h;
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true, antialias: false, alpha: false, depth: false, stencil: false });
  if (!gl) return null;
  const sh = (type, src) => {
    const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  let prog;
  try {
    prog = gl.createProgram();
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
  } catch (e) { console.warn("intro shader failed:", e.message); return null; }
  const uRes = gl.getUniformLocation(prog, "uRes"), uCx = gl.getUniformLocation(prog, "uCx"), uTheta = gl.getUniformLocation(prog, "uTheta");
  const vao = gl.createVertexArray();
  return {
    canvas,
    draw(t) {
      const b = ballState(w, h, t);
      gl.viewport(0, 0, w, h);
      gl.useProgram(prog); gl.bindVertexArray(vao);
      gl.uniform2f(uRes, w, h); gl.uniform1f(uCx, b.x); gl.uniform1f(uTheta, b.theta);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    },
    dispose() { gl.deleteProgram(prog); gl.deleteVertexArray(vao); gl.getExtension("WEBGL_lose_context")?.loseContext(); },
  };
}
