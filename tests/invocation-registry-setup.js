import { readFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

var __dirname_resolved = dirname(fileURLToPath(import.meta.url));

// Load shared.js first (invocation-registry depends on __companionUtils)
globalThis.window = globalThis;
globalThis.window.__companionUtils = globalThis.window.__companionUtils || {};
globalThis.window.__TAURI__ = null; // Prevent Tauri IPC calls in tests
globalThis.window.__rendererRegistry = {};
globalThis.document = {
  querySelectorAll: function() { return []; },
  createElement: function(tag) {
    var attrs = {};
    return {
      tagName: tag,
      className: '',
      textContent: '',
      setAttribute: function(k, v) { attrs[k] = v; },
      getAttribute: function(k) { return attrs[k] || null; },
    };
  }
};

var code = readFileSync(join(__dirname_resolved, '../public/renderers/invocation-registry.js'), 'utf8');
var fn = new Function(code);
fn.call(globalThis);
