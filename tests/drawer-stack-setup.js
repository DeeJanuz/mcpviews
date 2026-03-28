import { readFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

var __dirname_resolved = dirname(fileURLToPath(import.meta.url));

globalThis.window = globalThis;
globalThis.window.__companionUtils = globalThis.window.__companionUtils || {};
globalThis.window.__renderers = {};

// Mock DOM
var bodyChildren = [];
globalThis.document = {
  createElement: function(tag) {
    var attrs = {};
    var children = [];
    var classList = {
      _classes: new Set(),
      add: function(c) { this._classes.add(c); },
      remove: function(c) { this._classes.delete(c); },
      contains: function(c) { return this._classes.has(c); },
    };
    return {
      tagName: tag,
      className: '',
      textContent: '',
      style: {},
      classList: classList,
      children: children,
      _attrs: attrs,
      setAttribute: function(k, v) { attrs[k] = v; },
      getAttribute: function(k) { return attrs[k] || null; },
      appendChild: function(child) { children.push(child); },
      addEventListener: function(evt, handler) { this['_on' + evt] = handler; },
      parentNode: null,
    };
  },
  body: {
    _children: bodyChildren,
    appendChild: function(el) {
      el.parentNode = this;
      bodyChildren.push(el);
    },
    removeChild: function(el) {
      var idx = bodyChildren.indexOf(el);
      if (idx >= 0) bodyChildren.splice(idx, 1);
      el.parentNode = null;
    },
  },
  addEventListener: function() {},
};
globalThis.requestAnimationFrame = function(cb) { cb(); };
globalThis.setTimeout = function(cb) { cb(); };

var code = readFileSync(join(__dirname_resolved, '../public/renderers/drawer-stack.js'), 'utf8');
var fn = new Function(code);
fn.call(globalThis);
