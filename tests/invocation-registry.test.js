import './invocation-registry-setup.js';
import { describe, it, expect, beforeEach } from 'vitest';

// The invocation-registry.js registers populateRendererRegistry and autoDetectLinks on __companionUtils
// We need to test the globToRegex function indirectly through autoDetectLinks

describe('autoDetectLinks', function () {
  // autoDetectLinks is exposed on window.__companionUtils
  var autoDetectLinks = window.__companionUtils.autoDetectLinks;

  it('is a function', function () {
    expect(typeof autoDetectLinks).toBe('function');
  });

  it('does nothing with null container', function () {
    autoDetectLinks(null);
    // Should not throw
  });

  it('does nothing with empty registry', function () {
    window.__rendererRegistry = {};
    var container = {
      querySelectorAll: function() { return []; }
    };
    autoDetectLinks(container);
    // Should not throw
  });

  it('converts matching links to invocation buttons', function () {
    window.__rendererRegistry = {
      decision_detail: {
        display_mode: 'drawer',
        invoke_schema: '{ id: string }',
        url_patterns: ['/decisions/*'],
        plugin: 'test-plugin',
      }
    };

    var replaced = false;
    var mockLink = {
      getAttribute: function(name) {
        if (name === 'href') return '/decisions/dec-123';
        return null;
      },
      textContent: 'Decision 123',
      parentNode: {
        replaceChild: function(newEl, oldEl) {
          replaced = true;
          expect(newEl.getAttribute('data-invoke-renderer')).toBe('decision_detail');
          expect(newEl.textContent).toBe('Decision 123');
        }
      }
    };

    var container = {
      querySelectorAll: function(selector) {
        if (selector === 'a[href]') return [mockLink];
        return [];
      }
    };

    autoDetectLinks(container);
    expect(replaced).toBe(true);
  });

  it('does not convert non-matching links', function () {
    window.__rendererRegistry = {
      decision_detail: {
        display_mode: 'drawer',
        invoke_schema: '{ id: string }',
        url_patterns: ['/decisions/*'],
        plugin: 'test-plugin',
      }
    };

    var replaced = false;
    var mockLink = {
      getAttribute: function(name) {
        if (name === 'href') return '/users/user-456';
        return null;
      },
      textContent: 'User 456',
      parentNode: {
        replaceChild: function() { replaced = true; }
      }
    };

    var container = {
      querySelectorAll: function(selector) {
        if (selector === 'a[href]') return [mockLink];
        return [];
      }
    };

    autoDetectLinks(container);
    expect(replaced).toBe(false);
  });

  it('handles full URLs by matching pathname', function () {
    window.__rendererRegistry = {
      task_detail: {
        display_mode: 'drawer',
        invoke_schema: '{ id: string }',
        url_patterns: ['/tasks/*'],
        plugin: 'test-plugin',
      }
    };

    var replacedParams = null;
    var mockLink = {
      getAttribute: function(name) {
        if (name === 'href') return 'https://example.com/tasks/task-789';
        return null;
      },
      textContent: 'Task 789',
      parentNode: {
        replaceChild: function(newEl) {
          replacedParams = newEl.getAttribute('data-invoke-params');
        }
      }
    };

    var container = {
      querySelectorAll: function(selector) {
        if (selector === 'a[href]') return [mockLink];
        return [];
      }
    };

    autoDetectLinks(container);
    expect(replacedParams).not.toBeNull();
    var params = JSON.parse(replacedParams);
    expect(params.url).toBe('https://example.com/tasks/task-789');
  });
});

describe('populateRendererRegistry', function () {
  it('is a function', function () {
    expect(typeof window.__companionUtils.populateRendererRegistry).toBe('function');
  });

  it('resolves without error when TAURI is not available', async function () {
    window.__TAURI__ = null;
    await window.__companionUtils.populateRendererRegistry();
    // Should not throw, registry stays as-is
  });
});
