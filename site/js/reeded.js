// Fixed full-viewport "reeded glass" background. A soft, motion-smeared gradient (sky blue, coral, magenta,
// one deep-blue mass) with faint memory cells that pulse and recall, seen through vertical glass flutes.
// Technique: sine/lens-based UV offset per flute, ~12% zoom so edges do not smear, rib-edge highlights,
// per-channel offset (chromatic aberration). Raw WebGL1, no dependencies. Falls back to the CSS mesh.
const VERT = 'attribute vec2 p;varying vec2 v;void main(){v=p*.5+.5;gl_Position=vec4(p,0.,1.);}';
const FRAG = `precision highp float;
varying vec2 v;
uniform vec2 uRes;uniform float uT,uScroll,uFlutes;uniform vec2 uMouse;
float h(float n){return fract(sin(n*127.1)*43758.5453);}
vec3 scene(vec2 uv,float t){
  float sc=uScroll;
  float asp=uRes.x/uRes.y;
  vec2 q=uv;q.x*=asp;
  vec3 top=vec3(.13,.58,.93),mid=vec3(.26,.74,.96),low=vec3(.45,.82,.98);
  vec3 c=mix(mid,top,smoothstep(.2,1.,uv.y));c=mix(c,low,smoothstep(.0,.5,1.-uv.y)*.55);
  vec2 a=vec2(.62+.07*sin(t*.11),.42-sc*.55+.05*cos(t*.13));
  float coral=exp(-pow((q.x-a.x*asp)*1.15,2.)-pow((uv.y-a.y)*2.3,2.));
  vec2 b=vec2(.46+.06*cos(t*.09),.36-sc*.35+.04*sin(t*.12));
  float mag=exp(-pow((q.x-b.x*asp)*1.5,2.)-pow((uv.y-b.y)*3.1,2.));
  vec2 d=vec2(.58+.05*sin(t*.08+1.),.62-sc*.25);
  float navy=exp(-pow((q.x-d.x*asp)*3.6,2.)-pow((uv.y-d.y)*1.25,2.));
  c=mix(c,vec3(1.,.56,.46),clamp(coral*1.4,0.,1.));
  c=mix(c,vec3(.93,.25,.56),clamp(mag*1.1,0.,1.));
  c=mix(c,vec3(.07,.20,.52),clamp(navy*.55,0.,1.));
  for(int i=0;i<14;i++){
    float fi=float(i);
    vec2 o=vec2(.12+h(fi)*.8,.12+h(fi+9.)*.76-sc*.2);
    float ph=fract(t*.07+h(fi+3.)*3.);
    float wake=smoothstep(.0,.12,ph)*(1.-smoothstep(.12,.7,ph));
    vec2 dv=vec2(q.x-o.x*asp,uv.y-o.y);
    float r=length(dv);
    float cell=exp(-r*r*900.)*(.25+.75*wake);
    float ring=exp(-pow((r-ph*.35)*40.,2.))*(1.-ph)*.6;
    c+=vec3(1.,.82,.74)*cell*.55+vec3(1.,.7,.62)*ring*.25;
  }
  return c;
}
vec3 flutedSample(vec2 uv,float t,float shift){
  float W=uFlutes,x=uv.x*W,rib=floor(x),f=fract(x);
  float lens=(f-.5);
  vec2 s=vec2((rib+.5)/W-lens*(.9/W)*(7.+3.*sin(rib*.7+t*.2))+shift,uv.y);
  s.y+=(h(rib)-.5)*.012+sin(rib*.31+t*.15)*.006;
  s=(s-.5)*.88+.5;
  vec3 acc=vec3(0.);
  for(int k=-2;k<=2;k++){acc+=scene(s+vec2(float(k)*.006,0.),t);}
  return acc/5.;
}
void main(){
  vec2 uv=v;float t=uT;
  vec3 col;
  col.r=flutedSample(uv,t,.0016).r;col.g=flutedSample(uv,t,0.).g;col.b=flutedSample(uv,t,-.0016).b;
  float f=fract(uv.x*uFlutes);
  float edge=smoothstep(.0,.07,f)*(1.-smoothstep(.93,1.,f));
  col*=.9+.1*sin(f*3.14159);
  col+=vec3(.10)*(1.-smoothstep(.0,.09,f))+vec3(-.04)*smoothstep(.9,1.,f);
  col=mix(col,col*.84,(1.-edge)*.3);
  col+=(h(dot(floor(gl_FragCoord.xy),vec2(1.3,7.1))+t)-.5)*.03;
  col*=1.-.16*smoothstep(.55,0.,uv.y);
  gl_FragColor=vec4(clamp(col,0.,1.),1.);
}`;

export function createReeded({ reduce = false } = {}) {
  const canvas = document.createElement('canvas');
  canvas.className = 'rd-bg';
  canvas.setAttribute('aria-hidden', 'true');
  const gl = canvas.getContext('webgl', { antialias: false, alpha: false, powerPreference: 'low-power' });
  if (!gl) return null;
  const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null; };
  const vs = sh(gl.VERTEX_SHADER, VERT), fs = sh(gl.FRAGMENT_SHADER, FRAG);
  if (!vs || !fs) return null;
  const prog = gl.createProgram();
  gl.attachShader(prog, vs); gl.attachShader(prog, fs); gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return null;
  gl.useProgram(prog);
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  const loc = gl.getAttribLocation(prog, 'p');
  gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
  const U = (n) => gl.getUniformLocation(prog, n);
  const u = { res: U('uRes'), t: U('uT'), scroll: U('uScroll'), flutes: U('uFlutes'), mouse: U('uMouse') };
  const S = { t: 0, scroll: 0, on: true, mx: 0.5, my: 0.5 };

  const draw = () => {
    gl.uniform1f(u.t, S.t); gl.uniform1f(u.scroll, S.scroll); gl.uniform2f(u.mouse, S.mx, S.my);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  };
  const size = () => {
    const dpr = Math.min(devicePixelRatio || 1, 1.25);
    const w = Math.max(1, Math.round(innerWidth * dpr)), h = Math.max(1, Math.round(innerHeight * dpr));
    canvas.width = w; canvas.height = h;
    gl.viewport(0, 0, w, h);
    gl.uniform2f(u.res, w, h);
    gl.uniform1f(u.flutes, Math.max(24, Math.round(innerWidth / 9)));
    draw();
  };
  addEventListener('resize', size, { passive: true });
  addEventListener('scroll', () => {
    const hgt = document.documentElement.scrollHeight - innerHeight;
    S.scroll = hgt > 0 ? Math.min(1, scrollY / hgt) : 0;
    if (reduce) draw();
  }, { passive: true });
  addEventListener('pointermove', (e) => { S.mx = e.clientX / innerWidth; S.my = 1 - e.clientY / innerHeight; }, { passive: true });
  document.addEventListener('visibilitychange', () => { S.on = !document.hidden; });
  let last = 0;
  const loop = (now) => {
    requestAnimationFrame(loop);
    const dt = last ? Math.min(64, now - last) : 16; last = now;
    if (!S.on) return;
    S.t += dt / 1000;
    draw();
  };
  document.body.prepend(canvas);
  size();
  if (!reduce) requestAnimationFrame(loop);
  return { canvas };
}
