import { readFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

var __dirname_resolved = dirname(fileURLToPath(import.meta.url));

// Minimal marked mock that just calls the renderer.link function
globalThis.window = globalThis;
globalThis.document = {
  createElement: function(tag) {
    var _textContent = '';
    return {
      className: '',
      innerHTML: '',
      get textContent() { return _textContent; },
      set textContent(val) {
        _textContent = val;
        // Simulate real DOM: setting textContent makes innerHTML the escaped version
        this.innerHTML = String(val)
          .replace(/&/g, '&amp;')
          .replace(/</g, '&lt;')
          .replace(/>/g, '&gt;')
          .replace(/"/g, '&quot;');
      },
      appendChild: function() {},
    };
  }
};

// Mock marked to capture link renderer
var capturedRenderer = null;
globalThis.marked = {
  Renderer: function() {},
  setOptions: function(opts) { capturedRenderer = opts.renderer; },
  parse: function(text) { return text; },
};

// Mock fetch to prevent checkProxyStatus from making real requests
globalThis.fetch = function() { return Promise.resolve({ json: function() { return Promise.resolve({}); } }); };

var code = readFileSync(join(__dirname_resolved, '../public/renderers/shared.js'), 'utf8');
var fn = new Function(code);
fn.call(globalThis);

// Call renderMarkdown once to trigger marked.setOptions which captures the renderer
window.__companionUtils.renderMarkdown('init');

// Expose the captured renderer for testing
globalThis.__testRenderer = capturedRenderer;
