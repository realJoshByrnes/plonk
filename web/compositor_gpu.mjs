// WebGL2 port of the CPU glass compositor (crates/display/src/lib.rs compose_desktop/draw_surface/blit_surface).
//   const c = createGpuCompositor(width, height);  // null if WebGL2 is unavailable
//   c.render(layers);                              // layers bottom->top: { id, x, y, w, h, radius, blur, shadow, ver, pix(Uint32Array ARGB) }
//   c.canvas                                       // drawImage()-able right after render()
// Pipeline per frame: wallpaper -> for each layer { shadow quad; if blur>0: downsample backdrop 4x, 3x(box H + box V), then glass quad }
// -> blit to the canvas (y flipped). Layer textures are cached by id and re-uploaded only when ver/size changes.

const VS = `#version 300 es
uniform vec4 uRect; uniform vec2 uScreen;
void main(){ vec2 c=vec2(float(gl_VertexID&1),float(gl_VertexID>>1)); vec2 p=mix(uRect.xy,uRect.zw,c); gl_Position=vec4(p/uScreen*2.-1.,0.,1.); }`;

const HEAD = `#version 300 es
precision highp float; precision highp int; precision highp sampler2D;
uniform vec2 uScreen; out vec4 o;`;

const FS_WALL = HEAD + `
void main(){
  vec2 p=floor(gl_FragCoord.xy); float w=uScreen.x,h=uScreen.y; float u=p.x/w,v=p.y/h;
  float t=clamp(u*0.55+v*0.45,0.,1.);
  vec3 s0=vec3(96.,128.,220.),s1=vec3(168.,128.,226.),s2=vec3(240.,156.,190.);
  vec3 a=t<0.55?s0:s1, b=t<0.55?s1:s2; float a0=t<0.55?0.:0.55, b0=t<0.55?0.55:1.;
  float k=clamp((t-a0)/(b0-a0),0.,1.);
  vec3 c=mix(a,b,k);
  vec2 cs[2]=vec2[2](vec2(0.22,0.78),vec2(0.82,0.22)); float rs[2]=float[2](0.35,0.30);
  vec3 cols[2]=vec3[2](vec3(60.,210.,230.),vec3(255.,200.,120.));
  for(int i=0;i<2;i++){
    float d=length(vec2((u-cs[i].x)*w,(v-cs[i].y)*h))/(rs[i]*w);
    float g=pow(clamp(1.-d,0.,1.),2.)*0.55; c=c*(1.-g)+cols[i]*g;
  }
  o=vec4(floor(c)/255.,1.);
}`;

const FS_SHADOW = HEAD + `
uniform vec4 uLayer; uniform float uShadow; uniform float uRadius;
void main(){
  vec2 d=floor(gl_FragCoord.xy); float off=floor(uShadow/3.);
  float r=max(uRadius,1.);
  float px=d.x-uLayer.x+.5, py=d.y-uLayer.y-off+.5;
  float cx=clamp(px,r,uLayer.z-r), cy=clamp(py,r,uLayer.w-r);
  float dist=length(vec2(px-cx,py-cy))-r;
  if(dist<=0.) discard;
  float a=0.32*pow(clamp(1.-dist/uShadow,0.,1.),2.);
  o=vec4(0.,0.,0.,a);
}`;

const FS_DOWN = HEAD + `
uniform sampler2D uSrc; uniform vec4 uLayer; uniform vec3 uInfo; // layer x,y,w,h ; info: f
void main(){
  ivec2 b=ivec2(gl_FragCoord.xy); int f=int(uInfo.x); ivec2 sz=ivec2(uLayer.zw); ivec2 scr=textureSize(uSrc,0);
  vec3 acc=vec3(0.);
  for(int oy=0;oy<4;oy++){ if(oy>=f) break; for(int ox=0;ox<4;ox++){ if(ox>=f) break;
    ivec2 s=min(b*f+ivec2(ox,oy),sz-1)+ivec2(uLayer.xy);
    s=clamp(s,ivec2(0),scr-1); acc+=texelFetch(uSrc,s,0).rgb; } }
  o=vec4(acc/float(f*f),1.);
}`;

const FS_BOX = HEAD + `
uniform sampler2D uSrc; uniform ivec2 uDim; uniform int uR; uniform int uHoriz;
void main(){
  ivec2 p=ivec2(gl_FragCoord.xy); int n=uHoriz==1?uDim.x:uDim.y; int i=uHoriz==1?p.x:p.y;
  int lo=max(i-uR,0), hi=min(i+uR,n-1); vec3 s=vec3(0.);
  for(int k=0;k<256;k++){ int j=lo+k; if(j>hi) break; s+=texelFetch(uSrc,uHoriz==1?ivec2(j,p.y):ivec2(p.x,j),0).rgb; }
  o=vec4(s/float(hi-lo+1),1.);
}`;

const FS_LAYER = HEAD + `
uniform sampler2D uTex; uniform sampler2D uBlur; uniform int uHasBlur;
uniform vec4 uLayer; uniform float uRadius; uniform vec3 uBinfo; // dw,dh,f
vec3 glass(vec3 c, vec2 d){
  vec3 c8=floor(c*255.+.5);
  float lum=0.299*c8.r+0.587*c8.g+0.114*c8.b;
  uint h=((uint(d.x)*73856093u)^(uint(d.y)*19349663u))&255u;
  float grain=(float(h)/255.-.5)*3.;
  return floor(clamp(vec3(lum)+(c8-lum)*1.25+6.+grain,0.,255.))/255.;
}
void main(){
  vec2 d=floor(gl_FragCoord.xy); ivec2 l=ivec2(d-uLayer.xy);
  vec4 t=texelFetch(uTex,l,0); vec4 src=t.bgra;
  float cov=1.;
  if(uRadius>0.){
    vec2 p=vec2(l)+.5; float r=uRadius;
    vec2 c=vec2(clamp(p.x,r,uLayer.z-r),clamp(p.y,r,uLayer.w-r));
    cov=clamp(.5-(length(p-c)-r),0.,1.);
  }
  if(cov<=0.) discard;
  float a=src.a;
  if(uHasBlur==1){
    float f=uBinfo.z; vec2 fp=clamp((vec2(l)+.5)/f-.5,vec2(0.),uBinfo.xy-1.);
    vec3 b=texture(uBlur,(fp+.5)/vec2(textureSize(uBlur,0))).rgb;
    vec3 back=glass(b,d);
    o=vec4(mix(back,src.rgb,a),cov);
  } else o=vec4(src.rgb,a*cov);
}`;

const FS_BLIT = HEAD + `
uniform sampler2D uSrc;
void main(){ ivec2 p=ivec2(gl_FragCoord.xy); o=vec4(texelFetch(uSrc,ivec2(p.x,int(uScreen.y)-1-p.y),0).rgb,1.); }`;

export function createGpuCompositor(width, height) {
  let canvas;
  try { canvas = typeof document !== "undefined" ? document.createElement("canvas") : new OffscreenCanvas(width, height); } catch { return null; }
  canvas.width = width; canvas.height = height;
  let gl;
  try { gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true, antialias: false, alpha: false, depth: false, stencil: false, powerPreference: "high-performance" }); } catch { return null; }
  if (!gl) return null;
  const hasCBF = !!gl.getExtension("EXT_color_buffer_float"), hasCBH = !!gl.getExtension("EXT_color_buffer_half_float");
  const blurFmt = (hasCBF || hasCBH) ? { i: gl.RGBA16F, t: gl.HALF_FLOAT } : { i: gl.RGBA8, t: gl.UNSIGNED_BYTE };

  const compile = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)); return s; };
  const prog = (fs) => {
    const p = gl.createProgram(); gl.attachShader(p, compile(gl.VERTEX_SHADER, VS)); gl.attachShader(p, compile(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    const u = {}; const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
    for (let i = 0; i < n; i++) { const nm = gl.getActiveUniform(p, i).name; u[nm] = gl.getUniformLocation(p, nm); }
    return { p, u };
  };
  let P;
  try { P = { wall: prog(FS_WALL), shadow: prog(FS_SHADOW), down: prog(FS_DOWN), box: prog(FS_BOX), layer: prog(FS_LAYER), blit: prog(FS_BLIT) }; } catch (e) { console.warn("compositor_gpu: shader error", e); return null; }

  const vao = gl.createVertexArray(); gl.bindVertexArray(vao);
  const makeTex = (iformat, fmt, type, w, h, filter) => {
    const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, iformat, w, h, 0, fmt, type, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  };
  const makeFbo = (tex) => { const f = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, f); gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0); return f; };

  const curTex = makeTex(gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, width, height, gl.NEAREST), curFbo = makeFbo(curTex);
  const bt = [0, 1].map(() => { const t = makeTex(blurFmt.i, gl.RGBA, blurFmt.t, width, height, gl.LINEAR); return { t, f: makeFbo(t) }; });

  const layerTex = new Map(); // id -> { tex, w, h, ver }
  const use = (pr) => { gl.useProgram(pr.p); return pr.u; };
  const quad = (u, x0, y0, x1, y1, sw, sh) => { gl.uniform4f(u.uRect, x0, y0, x1, y1); gl.uniform2f(u.uScreen, sw, sh); gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4); };
  const bindTex = (unit, tex) => { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, tex); };

  function upload(L) {
    let e = layerTex.get(L.id);
    const bytes = new Uint8Array(L.pix.buffer, L.pix.byteOffset, L.w * L.h * 4);
    gl.activeTexture(gl.TEXTURE0);
    if (!e) { e = { tex: gl.createTexture(), w: 0, h: 0, ver: -1 }; layerTex.set(L.id, e); gl.bindTexture(gl.TEXTURE_2D, e.tex);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE); }
    if (e.w !== L.w || e.h !== L.h) { gl.bindTexture(gl.TEXTURE_2D, e.tex); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, L.w, L.h, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes); e.w = L.w; e.h = L.h; e.ver = L.ver; }
    else if (e.ver !== L.ver) { gl.bindTexture(gl.TEXTURE_2D, e.tex); gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, L.w, L.h, gl.RGBA, gl.UNSIGNED_BYTE, bytes); e.ver = L.ver; }
    return e;
  }

  // Blur the backdrop under layer L (current contents of curTex). Result is in bt[0], valid region dw x dh.
  function blurBackdrop(L) {
    const f = L.blur >= 12 ? 4 : 1;
    const dw = Math.ceil(L.w / f), dh = Math.ceil(L.h / f);
    gl.disable(gl.BLEND);
    gl.viewport(0, 0, dw, dh);
    gl.bindFramebuffer(gl.FRAMEBUFFER, bt[0].f);
    let u = use(P.down); bindTex(0, curTex); gl.uniform1i(u.uSrc, 0); gl.uniform4f(u.uLayer, L.x, L.y, L.w, L.h); gl.uniform3f(u.uInfo, f, 0, 0);
    quad(u, 0, 0, dw, dh, dw, dh);
    const r = Math.min(255, Math.max(1, Math.floor(Math.floor(L.blur / 3) / f)));
    u = use(P.box); gl.uniform1i(u.uSrc, 0); gl.uniform2i(u.uDim, dw, dh); gl.uniform1i(u.uR, r);
    let a = 0;
    for (let i = 0; i < 6; i++) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, bt[1 - a].f); bindTex(0, bt[a].t);
      gl.uniform1i(u.uHoriz, i % 2 === 0 ? 1 : 0);
      quad(u, 0, 0, dw, dh, dw, dh); a = 1 - a;
    }
    return { dw, dh, f, tex: bt[a].t };
  }

  function render(layers) {
    gl.disable(gl.DEPTH_TEST); gl.disable(gl.CULL_FACE);
    const seen = new Set();
    for (const L of layers) { if (L.w > 0 && L.h > 0 && L.pix) { upload(L); seen.add(L.id); } }
    for (const [id, e] of layerTex) if (!seen.has(id)) { gl.deleteTexture(e.tex); layerTex.delete(id); }

    gl.bindFramebuffer(gl.FRAMEBUFFER, curFbo); gl.viewport(0, 0, width, height); gl.disable(gl.BLEND);
    let u = use(P.wall); quad(u, 0, 0, width, height, width, height);

    for (const L of layers) {
      if (!seen.has(L.id)) continue;
      const e = layerTex.get(L.id);
      if (L.shadow > 0) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, curFbo); gl.viewport(0, 0, width, height);
        gl.enable(gl.BLEND); gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.ZERO, gl.ONE);
        u = use(P.shadow); gl.uniform4f(u.uLayer, L.x, L.y, L.w, L.h); gl.uniform1f(u.uShadow, L.shadow); gl.uniform1f(u.uRadius, L.radius);
        quad(u, L.x - L.shadow, L.y - L.shadow, L.x + L.w + L.shadow, L.y + L.h + L.shadow + Math.floor(L.shadow / 3), width, height);
      }
      let bl = null;
      if (L.blur > 0) bl = blurBackdrop(L);
      gl.bindFramebuffer(gl.FRAMEBUFFER, curFbo); gl.viewport(0, 0, width, height);
      gl.enable(gl.BLEND); gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.ZERO, gl.ONE);
      u = use(P.layer);
      bindTex(0, e.tex); gl.uniform1i(u.uTex, 0);
      bindTex(1, bl ? bl.tex : e.tex); gl.uniform1i(u.uBlur, 1);
      gl.uniform1i(u.uHasBlur, bl ? 1 : 0);
      gl.uniform4f(u.uLayer, L.x, L.y, L.w, L.h); gl.uniform1f(u.uRadius, L.radius);
      gl.uniform3f(u.uBinfo, bl ? bl.dw : 0, bl ? bl.dh : 0, bl ? bl.f : 1);
      quad(u, L.x, L.y, L.x + L.w, L.y + L.h, width, height);
    }

    gl.disable(gl.BLEND); gl.bindFramebuffer(gl.FRAMEBUFFER, null); gl.viewport(0, 0, width, height);
    u = use(P.blit); bindTex(0, curTex); gl.uniform1i(u.uSrc, 0);
    quad(u, 0, 0, width, height, width, height);
  }

  return { canvas, render, gl };
}
