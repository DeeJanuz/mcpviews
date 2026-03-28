import './drawer-stack-setup.js';
import { describe, it, expect, beforeEach } from 'vitest';

var utils = window.__companionUtils;

describe('drawer-stack', function () {
  beforeEach(function () {
    // Close all drawers and clear body children between tests
    utils.closeAllDrawers();
    document.body._children.length = 0;
  });

  it('exposes invokeRenderer, closeDrawer, closeAllDrawers', function () {
    expect(typeof utils.invokeRenderer).toBe('function');
    expect(typeof utils.closeDrawer).toBe('function');
    expect(typeof utils.closeAllDrawers).toBe('function');
  });

  it('adds overlay and panel to body on invokeRenderer', function () {
    utils.invokeRenderer('test_renderer', { id: '123' });
    // Should have added 2 elements: overlay + panel
    expect(document.body._children.length).toBe(2);
    expect(document.body._children[0].className).toBe('drawer-stack-overlay');
    expect(document.body._children[1].className).toBe('drawer-stack-panel');
  });

  it('shows renderer not found message for unknown renderer', function () {
    utils.invokeRenderer('nonexistent_renderer', {});
    var panel = document.body._children[1]; // panel
    var content = panel.children[1]; // second child is content div
    expect(content.textContent).toBe('Renderer not found: nonexistent_renderer');
  });

  it('calls renderer when found', function () {
    var called = false;
    var receivedParams = null;
    var receivedContext = null;
    window.__renderers.mock_renderer = function(container, params, a, b, c, d, context) {
      called = true;
      receivedParams = params;
      receivedContext = context;
    };

    utils.invokeRenderer('mock_renderer', { id: 'test' });
    expect(called).toBe(true);
    expect(receivedParams).toEqual({ id: 'test' });
    expect(receivedContext.mode).toBe('drawer');
    expect(receivedContext.level).toBe(0);
    expect(typeof receivedContext.invoke).toBe('function');

    delete window.__renderers.mock_renderer;
  });

  it('closeDrawer removes topmost overlay and panel', function () {
    utils.invokeRenderer('test1', {});
    expect(document.body._children.length).toBe(2);

    utils.closeDrawer();
    expect(document.body._children.length).toBe(0);
  });

  it('stacks multiple drawers', function () {
    utils.invokeRenderer('first', {});
    utils.invokeRenderer('second', {});
    // 2 overlays + 2 panels = 4 children
    expect(document.body._children.length).toBe(4);
  });

  it('closeAllDrawers removes all drawers', function () {
    utils.invokeRenderer('first', {});
    utils.invokeRenderer('second', {});
    utils.invokeRenderer('third', {});
    expect(document.body._children.length).toBe(6);

    utils.closeAllDrawers();
    expect(document.body._children.length).toBe(0);
  });

  it('closeDrawer does nothing when stack is empty', function () {
    utils.closeDrawer(); // Should not throw
    expect(document.body._children.length).toBe(0);
  });

  it('sets increasing z-index per level', function () {
    utils.invokeRenderer('first', {});
    utils.invokeRenderer('second', {});

    // First overlay z=150, first panel z=151
    expect(document.body._children[0].style.zIndex).toBe('150');
    expect(document.body._children[1].style.zIndex).toBe('151');
    // Second overlay z=152, second panel z=153
    expect(document.body._children[2].style.zIndex).toBe('152');
    expect(document.body._children[3].style.zIndex).toBe('153');
  });

  it('sets decreasing width per level', function () {
    utils.invokeRenderer('first', {});
    utils.invokeRenderer('second', {});

    // First panel width: 420px, second: 400px
    expect(document.body._children[1].style.width).toBe('420px');
    expect(document.body._children[3].style.width).toBe('400px');
  });

  it('increments context level for nested invocations', function () {
    var levels = [];
    window.__renderers.level_test = function(container, params, a, b, c, d, context) {
      levels.push(context.level);
    };

    utils.invokeRenderer('level_test', {});
    utils.invokeRenderer('level_test', {});
    utils.invokeRenderer('level_test', {});

    expect(levels).toEqual([0, 1, 2]);
    delete window.__renderers.level_test;
  });
});
