//! Web-only JSX helpers for `tish compile --target js`.
//! Native and WASM native paths must not depend on this crate.

/// VDOM: vnode `__vdom_h` + `window.__lattishVdomPatch` for Lattish batched flush.
pub const VDOM_PRELUDE: &str = r#"window.__LATTISH_JSX_VDOM = true;
const __Fragment = Symbol('Fragment');
function __vdom_h(tag, props, children) {
  if (children === undefined || children === null) children = [];
  if (!Array.isArray(children)) children = [children];
  return { tag: tag, props: props || null, children: children, _el: null };
}
function __vdom_flatten(ch) {
  const out = [];
  function w(c) {
    if (c == null) return;
    if (Array.isArray(c)) { for (let i = 0; i < c.length; i++) w(c[i]); return; }
    if (typeof c === 'object' && c && c.tag === __Fragment) {
      const inner = c.children;
      if (Array.isArray(inner)) for (let i = 0; i < inner.length; i++) w(inner[i]);
      return;
    }
    out.push(c);
  }
  if (Array.isArray(ch)) for (let i = 0; i < ch.length; i++) w(ch[i]);
  return out;
}
function __vdom_mount(v) {
  if (typeof v === 'string') return document.createTextNode(v);
  if (v.tag === __Fragment) {
    const f = document.createDocumentFragment();
    const ch = __vdom_flatten(v.children);
    for (let i = 0; i < ch.length; i++) f.appendChild(__vdom_mount(ch[i]));
    return f;
  }
  const el = document.createElement(v.tag);
  const p = v.props || {};
  for (const k of Object.keys(p)) {
    const val = p[k];
    if (val === true) el.setAttribute(k, k);
    else if (val !== false && val != null) {
      if (k === 'class' || k === 'className') el.className = val;
      else if (k.startsWith('on') && typeof val === 'function') el[k.toLowerCase()] = val;
      else if (k === 'value' && (v.tag === 'input' || v.tag === 'textarea' || v.tag === 'select')) el.value = val;
      else el.setAttribute(k, String(val));
    }
  }
  const ch = __vdom_flatten(v.children);
  for (let i = 0; i < ch.length; i++) el.appendChild(__vdom_mount(ch[i]));
  v._el = el;
  return el;
}
function __vdomPatchEl(el, ov, nv) {
  if (!el || !ov || !nv) return false;
  if (typeof nv === 'string') {
    if (el.nodeType === 3) el.textContent = nv;
    return true;
  }
  if (ov.tag !== nv.tag) return false;
  nv._el = el;
  const p = nv.props || {};
  const op = ov.props || {};
  for (const k of Object.keys(p)) {
    if (p[k] !== op[k]) {
      const val = p[k];
      if (k === 'class' || k === 'className') el.className = val || '';
      else if (k.startsWith('on')) el[k.toLowerCase()] = val || null;
      else if (k === 'value' && (nv.tag === 'input' || nv.tag === 'textarea' || nv.tag === 'select')) {
        if (el !== document.activeElement) el.value = val;
      } else el.setAttribute(k, String(val));
    }
  }
  const ocx = __vdom_flatten(ov.children);
  const ncx = __vdom_flatten(nv.children);
  let fullCh = ocx.length !== ncx.length;
  if (!fullCh) {
    for (let j = 0; j < ncx.length; j++) {
      const o = ocx[j], n = ncx[j];
      if (typeof n === 'string') { if (typeof o !== 'string') fullCh = true; }
      else if (typeof o === 'string') fullCh = true;
      else if (!o || o.tag !== n.tag) fullCh = true;
      if (fullCh) break;
    }
  }
  if (fullCh) {
    const nodes = [];
    for (let j = 0; j < ncx.length; j++) nodes.push(__vdom_mount(ncx[j]));
    el.replaceChildren(...nodes);
    return true;
  }
  let i = 0;
  while (i < ncx.length) {
    const o = ocx[i], n = ncx[i];
    const childEl = el.childNodes[i];
    if (n == null) {
      if (childEl) el.removeChild(childEl);
      i++;
      continue;
    }
    if (o == null) {
      el.appendChild(__vdom_mount(n));
      i++;
      continue;
    }
    if (typeof n === 'string') {
      if (childEl && childEl.nodeType === 3) childEl.textContent = n;
      else {
        const m = __vdom_mount(n);
        if (childEl) el.replaceChild(m, childEl);
        else el.appendChild(m);
      }
      i++;
      continue;
    }
    if (typeof o === 'string' || o.tag !== n.tag) {
      const m = __vdom_mount(n);
      if (childEl) el.replaceChild(m, childEl);
      else el.appendChild(m);
      i++;
      continue;
    }
    if (!childEl || !__vdomPatchEl(childEl, o, n)) {
      const m = __vdom_mount(n);
      if (childEl) el.replaceChild(m, childEl);
      else el.appendChild(m);
    }
    i++;
  }
  return true;
}
window.__lattishVdomPatch = function(container, oldTree, newTree) {
  try {
    if (oldTree == null) {
      container.replaceChildren(__vdom_mount(newTree));
      return;
    }
    const el = container.firstChild;
    if (!el) {
      container.appendChild(__vdom_mount(newTree));
      return;
    }
    if (typeof newTree === 'string' || oldTree.tag !== newTree.tag) {
      container.replaceChildren(__vdom_mount(newTree));
      return;
    }
    if (!__vdomPatchEl(el, oldTree, newTree)) {
      container.replaceChildren(__vdom_mount(newTree));
    }
  } catch (err) {
    console.warn('Lattish VDOM patch failed, full remount', err);
    try {
      container.replaceChildren(__vdom_mount(newTree));
    } catch (e2) {
      console.error('Lattish VDOM remount failed', e2);
    }
  }
};
"#;
