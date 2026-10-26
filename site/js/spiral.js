// GPU spiral: text is baked into one texture, then rotated / twisted / zoomed in a fragment shader.
// Per-frame cost is one fullscreen quad; no 2D text drawing after startup.
import * as THREE from '../vendor/three.module.min.js';

const SIZE = 4096;        // texture px
const SPAN = 1.9;         // texture covers SPAN view-heights
const PITCH_PX = 74;      // radial gap between spiral turns, in texture px
const R0_PX = 26;
const WORD = 'PROJECT APEX · ATLAS · ';
const PITCH = (PITCH_PX / SIZE) * SPAN;
const R0 = (R0_PX / SIZE) * SPAN;

const VERT = 'varying vec2 vUv;void main(){vUv=uv;gl_Position=vec4(position.xy,0.,1.);}';
const FRAG = `
precision highp float;
uniform sampler2D tMap;
uniform float uAspect,uZoom,uRot,uTime,uTwist,uShow,uPx;
uniform vec2 uCenter,uPointer;
uniform vec3 uBg,uInk;
varying vec2 vUv;
const float SPAN=${SPAN.toFixed(4)};
const float PITCH=${PITCH.toFixed(6)};
const float R0=${R0.toFixed(6)};
void main(){
  vec2 p=(vUv-uCenter)*vec2(uAspect,1.0);
  p-=uPointer*0.022;
  p/=uZoom;
  float r=length(p);
  float a=atan(p.y,p.x);
  a+=uRot+uTwist*exp(-r*3.2)*sin(uTime*0.35);
  vec2 q=vec2(cos(a),sin(a))*r;
  float t=texture2D(tMap,q/SPAN+0.5).a;
  float fade=mix(1.0,0.38,smoothstep(0.25,1.1,r*uZoom));
  // dotted guide rings, constant pixel width
  float rr=(r-R0)/PITCH;
  float dist=abs(fract(rr+0.5)-0.5)*PITCH;
  float ring=1.0-smoothstep(0.0,uPx*1.1/uZoom,dist);
  float dots=step(fract((a/6.28318)*floor(r/PITCH+0.5)*7.0),0.34);
  vec3 col=uBg;
  col+=uInk*ring*dots*0.2*uShow*step(0.02,r);
  col=mix(col,uInk,t*fade*uShow);
  gl_FragColor=vec4(col,1.0);
}`;

function bakeTexture() {
  const c = document.createElement('canvas');
  c.width = c.height = SIZE;
  const x = c.getContext('2d');
  x.fillStyle = '#fff';
  x.textAlign = 'center';
  x.textBaseline = 'middle';
  const max = (SIZE / 2) * 1.04;
  let th = 0, i = 0;
  for (;;) {
    const r = R0_PX + (PITCH_PX * th) / (Math.PI * 2);
    if (r > max) break;
    const fs = Math.min(40, 14 + r / 26);
    x.font = `500 ${fs.toFixed(1)}px "Geist Mono", monospace`;
    x.save();
    x.translate(SIZE / 2 + Math.cos(th) * r, SIZE / 2 + Math.sin(th) * r);
    x.rotate(th + Math.PI / 2);
    x.fillText(WORD[i % WORD.length], 0, 0);
    x.restore();
    th += (fs * 0.68) / r;
    i++;
  }
  const tex = new THREE.CanvasTexture(c);
  tex.generateMipmaps = true;
  tex.minFilter = THREE.LinearMipmapLinearFilter;
  tex.magFilter = THREE.LinearFilter;
  tex.anisotropy = 8;
  return tex;
}

/** @param {HTMLElement} host element containing a <canvas>. @param {{ticker:any,reduce:boolean}} o */
export async function createSpiral(host, { ticker, reduce }) {
  const canvas = host.querySelector('canvas');
  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: false, powerPreference: 'high-performance' });
  } catch {
    return null; // no WebGL: the dark panel still reads fine
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  const scene = new THREE.Scene();
  const cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const u = {
    tMap: { value: null }, uAspect: { value: 1 }, uZoom: { value: 1 }, uRot: { value: 0 }, uTime: { value: 0 },
    uTwist: { value: 0.55 }, uShow: { value: 0 }, uPx: { value: 0.0014 },
    uCenter: { value: new THREE.Vector2(0.75, 0.5) }, uPointer: { value: new THREE.Vector2() },
    uBg: { value: new THREE.Color('#232323') }, uInk: { value: new THREE.Color('#f1eee7') },
  };
  const mat = new THREE.ShaderMaterial({ uniforms: u, vertexShader: VERT, fragmentShader: FRAG, depthTest: false, depthWrite: false });
  scene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), mat));

  const size = () => {
    const w = host.clientWidth, h = host.clientHeight;
    if (!w || !h) return;
    renderer.setSize(w, h, false);
    u.uAspect.value = w / h;
    u.uPx.value = 1 / (h * renderer.getPixelRatio());
    render();
  };
  const render = () => renderer.render(scene, cam);

  const state = { zoom: 1, center: 0.75, paused: false, onscreen: true, px: 0, py: 0, rot: 0 };
  new ResizeObserver(size).observe(host);
  new IntersectionObserver((e) => { state.onscreen = e[0].isIntersecting; }).observe(host);
  host.addEventListener('pointermove', (e) => {
    const r = host.getBoundingClientRect();
    state.px = ((e.clientX - r.left) / r.width - 0.5) * 2;
    state.py = -((e.clientY - r.top) / r.height - 0.5) * 2;
  });

  ticker.add((time, dt) => {
    if (state.paused || !state.onscreen || !u.tMap.value) return;
    if (!reduce) {
      state.rot += dt * 0.00005;
      u.uTime.value = time;
    }
    u.uRot.value = state.rot;
    u.uPointer.value.x += (state.px - u.uPointer.value.x) * 0.06;
    u.uPointer.value.y += (state.py - u.uPointer.value.y) * 0.06;
    u.uZoom.value = state.zoom;
    u.uCenter.value.x = state.center;
    render();
  });

  size();
  // bake after fonts + first paint so the page is interactive immediately
  await document.fonts.load('500 20px "Geist Mono"');
  await new Promise((r) => (window.requestIdleCallback ? requestIdleCallback(r, { timeout: 400 }) : setTimeout(r, 60)));
  u.tMap.value = bakeTexture();
  size();

  return {
    /** dive progress 0..1: zoom in and slide the spiral centre from the right half to the middle */
    dive(p) { state.zoom = 1 + p * 1.55; state.center = 0.75 - p * 0.25; },
    pause(v) { state.paused = v; },
    show(v) { u.uShow.value = v; if (reduce) render(); },
    uniforms: u,
  };
}
